use std::collections::VecDeque;
use std::hash::Hash;

use async_trait::async_trait;
use bounded_integer::BoundedU8;
use chrono::{Duration, Utc};
use priority_queue::PriorityQueue;
use rand::Rng;

use crate::backlog::data::{BacklogOrder, OrderWeight, Weighted};
use crate::backlog::persistence::BacklogStore;
use crate::data::order::{PendingOrder, ProgressingOrder, SuspendedOrder};
use crate::data::OnChainOrder;

pub mod data;
pub mod persistence;
pub mod process;

/// Backlog manages orders on all stages of their life.
/// Usually in the order defined by some weighting function (e.g. orders with higher fee are preferred).
#[async_trait(?Send)]
pub trait Backlog<TOrd>
where
    TOrd: OnChainOrder,
{
    /// Add new pending order to backlog.
    async fn put(&mut self, ord: PendingOrder<TOrd>);
    /// Suspend order that temporarily failed.
    /// Potentially retry later.
    async fn suspend(&mut self, ord: TOrd) -> bool;
    /// Register successfull order to check if it settled later.
    async fn check_later(&mut self, ord: ProgressingOrder<TOrd>) -> bool;
    /// Pop best order.
    async fn try_pop(&mut self) -> Option<TOrd>;
    /// Remove order from backlog.
    async fn remove(&mut self, ord_id: TOrd::TOrderId);
    /// Return order back to backlog.
    async fn recharge(&mut self, ord: TOrd);
}

pub struct BacklogConfig {
    pub order_lifespan: Duration,
    pub order_exec_time: Duration,
    pub retry_suspended_prob: BoundedU8<0, 100>,
}

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
struct WeightedOrder<TOrderId> {
    order_id: TOrderId,
    timestamp: i64,
}

impl<TOrd> From<BacklogOrder<TOrd>> for WeightedOrder<TOrd::TOrderId>
where
    TOrd: OnChainOrder,
{
    fn from(bo: BacklogOrder<TOrd>) -> Self {
        Self {
            order_id: bo.order.get_self_ref(),
            timestamp: bo.timestamp,
        }
    }
}

impl<TOrd> From<PendingOrder<TOrd>> for WeightedOrder<TOrd::TOrderId>
where
    TOrd: OnChainOrder,
{
    fn from(po: PendingOrder<TOrd>) -> Self {
        Self {
            order_id: po.order.get_self_ref(),
            timestamp: po.timestamp,
        }
    }
}

impl<TOrd> From<ProgressingOrder<TOrd>> for WeightedOrder<TOrd::TOrderId>
where
    TOrd: OnChainOrder,
{
    fn from(po: ProgressingOrder<TOrd>) -> Self {
        Self {
            order_id: po.order.get_self_ref(),
            timestamp: po.timestamp,
        }
    }
}

impl<TOrd> From<SuspendedOrder<TOrd>> for WeightedOrder<TOrd::TOrderId>
where
    TOrd: OnChainOrder,
{
    fn from(so: SuspendedOrder<TOrd>) -> Self {
        Self {
            order_id: so.order.get_self_ref(),
            timestamp: so.timestamp,
        }
    }
}

pub struct BacklogService<TOrd, TStore>
where
    TOrd: OnChainOrder + Hash + Eq,
{
    store: TStore,
    conf: BacklogConfig,
    /// Pending orders ordered by weight.
    pending_pq: PriorityQueue<WeightedOrder<TOrd::TOrderId>, OrderWeight>,
    /// Failed orders waiting for retry (retries are performed with some constant probability, e.g. 5%).
    /// Again, ordered by weight.
    suspended_pq: PriorityQueue<WeightedOrder<TOrd::TOrderId>, OrderWeight>,
    /// Successully submitted orders. Left orders should be re-executed in some time.
    /// Normally successfull orders are eliminated from this queue before new execution attempt.
    revisit_queue: VecDeque<WeightedOrder<TOrd::TOrderId>>,
}

impl<TOrd, TStore> BacklogService<TOrd, TStore>
where
    TOrd: OnChainOrder + Weighted + Hash + Eq,
    TStore: BacklogStore<TOrd>,
{
    pub fn new(store: TStore, conf: BacklogConfig) -> Self {
        Self {
            store,
            conf,
            pending_pq: PriorityQueue::new(),
            suspended_pq: PriorityQueue::new(),
            revisit_queue: VecDeque::new(),
        }
    }

    async fn revisit_progressing_orders(&mut self) {
        while let Some(ord) = self.revisit_queue.pop_front() {
            let ts_now = Utc::now().timestamp();
            let elapsed_secs = ts_now - ord.timestamp;
            if elapsed_secs > self.conf.order_exec_time.num_seconds() {
                if elapsed_secs <= self.conf.order_lifespan.num_seconds() {
                    if let Some(ord) = self.store.get(ord.order_id).await {
                        let wt = ord.order.weight();
                        self.pending_pq.push(ord.into(), wt);
                    }
                } else {
                    self.store.drop(ord.order_id).await;
                }
            } else {
                break;
            }
        }
    }
}

async fn try_pop_max_order<TOrd, TStore>(
    conf: &BacklogConfig,
    store: &mut TStore,
    pq: &mut PriorityQueue<WeightedOrder<TOrd::TOrderId>, OrderWeight>,
) -> Option<TOrd>
where
    TOrd: OnChainOrder + Weighted + Hash + Eq,
    TStore: BacklogStore<TOrd>,
{
    while let Some((ord, _)) = pq.pop() {
        let ts_now = Utc::now().timestamp();
        let elapsed_secs = ts_now - ord.timestamp;
        if elapsed_secs > conf.order_lifespan.num_seconds() {
            store.drop(ord.order_id).await;
        } else {
            return store.get(ord.order_id).await.map(|bo| bo.order);
        }
    }
    None
}

#[async_trait(?Send)]
impl<TOrd, TStore> Backlog<TOrd> for BacklogService<TOrd, TStore>
where
    TStore: BacklogStore<TOrd>,
    TOrd: OnChainOrder + Weighted + Hash + Eq + Clone,
{
    async fn put(&mut self, ord: PendingOrder<TOrd>) {
        self.store
            .put(BacklogOrder {
                order: ord.order.clone(),
                timestamp: ord.timestamp,
            })
            .await;
        let wt = ord.order.weight();
        self.pending_pq.push(ord.into(), wt);
    }

    async fn suspend(&mut self, ord: TOrd) -> bool {
        if self.store.exists(ord.get_self_ref()).await {
            let wt = ord.weight();
            if let Some(backlog_ord) = self.store.get(ord.get_self_ref()).await {
                self.suspended_pq.push(
                    WeightedOrder {
                        order_id: ord.get_self_ref(),
                        timestamp: backlog_ord.timestamp,
                    },
                    wt,
                );
                return true;
            }
        }
        false
    }

    async fn check_later(&mut self, ord: ProgressingOrder<TOrd>) -> bool {
        if self.store.exists(ord.order.get_self_ref()).await {
            self.revisit_queue.push_back(ord.into());
            return true;
        }
        false
    }

    async fn try_pop(&mut self) -> Option<TOrd> {
        self.revisit_progressing_orders().await;
        let rng = rand::thread_rng().gen_range(0..=99);
        if rng >= self.conf.retry_suspended_prob.get() {
            try_pop_max_order(&self.conf, &mut self.store, &mut self.pending_pq).await
        } else {
            try_pop_max_order(&self.conf, &mut self.store, &mut self.suspended_pq).await
        }
    }

    async fn remove(&mut self, ord_id: TOrd::TOrderId) {
        self.store.drop(ord_id).await;
    }

    async fn recharge(&mut self, ord: TOrd) {
        let wt = ord.weight();
        if let Some(backlog_ord) = self.store.get(ord.get_self_ref()).await {
            self.pending_pq.push(
                WeightedOrder {
                    order_id: ord.get_self_ref(),
                    timestamp: backlog_ord.timestamp,
                },
                wt,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Arc;

    use async_trait::async_trait;
    use bounded_integer::BoundedU8;
    use chrono::{Duration, Utc};
    use ergo_chain_sync::cache::rocksdb::RocksDBClient;
    use serde::{Deserialize, Serialize};
    use type_equalities::IsEqual;

    use crate::backlog::data::{BacklogOrder, OrderWeight, Weighted};
    use crate::backlog::persistence::BacklogStore;
    use crate::backlog::{Backlog, BacklogConfig, BacklogService};
    use crate::data::order::{PendingOrder, ProgressingOrder, SuspendedOrder};
    use crate::data::{Has, OnChainOrder};

    #[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Hash, Clone, Copy, Serialize, Deserialize)]
    struct MockOrderId(i64);

    #[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Hash, Clone, Serialize, Deserialize)]
    struct MockOrder {
        order_id: MockOrderId,
        weight: OrderWeight,
    }

    impl From<BacklogOrder<MockOrder>> for PendingOrder<MockOrder> {
        fn from(x: BacklogOrder<MockOrder>) -> Self {
            Self {
                order: x.order,
                timestamp: x.timestamp,
            }
        }
    }

    impl From<BacklogOrder<MockOrder>> for ProgressingOrder<MockOrder> {
        fn from(x: BacklogOrder<MockOrder>) -> Self {
            Self {
                order: x.order,
                timestamp: x.timestamp,
            }
        }
    }

    impl From<BacklogOrder<MockOrder>> for SuspendedOrder<MockOrder> {
        fn from(x: BacklogOrder<MockOrder>) -> Self {
            Self {
                order: x.order,
                timestamp: x.timestamp,
            }
        }
    }

    impl Has<MockOrderId> for MockOrder {
        fn get<U: IsEqual<MockOrderId>>(&self) -> MockOrderId {
            self.order_id
        }
    }

    impl Weighted for MockOrder {
        fn weight(&self) -> OrderWeight {
            self.weight
        }
    }

    impl OnChainOrder for MockOrder {
        type TOrderId = MockOrderId;
        type TEntityId = ();

        fn get_self_ref(&self) -> Self::TOrderId {
            self.order_id
        }

        fn get_entity_ref(&self) -> Self::TEntityId {}
    }

    struct MockBacklogStore {
        inner: HashMap<MockOrderId, BacklogOrder<MockOrder>>,
    }

    impl MockBacklogStore {
        fn new() -> Self {
            Self {
                inner: HashMap::new(),
            }
        }
    }

    #[async_trait(?Send)]
    impl BacklogStore<MockOrder> for MockBacklogStore {
        async fn put(&mut self, ord: BacklogOrder<MockOrder>) {
            self.inner.insert(ord.order.order_id, ord);
        }

        async fn exists(&self, ord_id: MockOrderId) -> bool {
            self.inner.contains_key(&ord_id)
        }

        async fn drop(&mut self, ord_id: MockOrderId) {
            self.inner.remove(&ord_id);
        }

        async fn get(&self, ord_id: MockOrderId) -> Option<BacklogOrder<MockOrder>> {
            self.inner.get(&ord_id).cloned()
        }
    }

    fn setup_backlog(
        order_lifespan_secs: i64,
        order_exec_time_secs: i64,
        retry_suspended_prob: u8,
    ) -> BacklogService<MockOrder, MockBacklogStore> {
        let store = MockBacklogStore::new();
        let conf = BacklogConfig {
            order_lifespan: Duration::seconds(order_lifespan_secs),
            order_exec_time: Duration::seconds(order_exec_time_secs),
            retry_suspended_prob: <BoundedU8<0, 100>>::new(retry_suspended_prob).unwrap(),
        };
        BacklogService::new(store, conf)
    }

    fn make_order(id: i64, weight: u64) -> BacklogOrder<MockOrder> {
        BacklogOrder {
            order: MockOrder {
                order_id: MockOrderId(id),
                weight: OrderWeight::from(weight),
            },
            timestamp: Utc::now().timestamp(),
        }
    }

    #[tokio::test]
    async fn should_suspend_existing_order() {
        let mut backlog = setup_backlog(10, 5, 50);
        let ord = make_order(1, 1);
        backlog.put(ord.clone().into()).await;
        let suspended = backlog.suspend(ord.order).await;
        assert!(suspended)
    }

    #[tokio::test]
    async fn should_check_later_existing_order() {
        let mut backlog = setup_backlog(10, 5, 50);
        let ord = make_order(1, 1);
        backlog.put(ord.clone().into()).await;
        let accepted = backlog.check_later(ord.into()).await;
        assert!(accepted)
    }

    #[tokio::test]
    async fn should_not_suspend_non_existent_order() {
        let mut backlog = setup_backlog(10, 5, 50);
        let ord = make_order(1, 1);
        let suspended = backlog.suspend(ord.order).await;
        assert!(!suspended)
    }

    #[tokio::test]
    async fn should_not_check_later_non_existent_order() {
        let mut backlog = setup_backlog(10, 5, 50);
        let ord = make_order(1, 1);
        let accepted = backlog
            .check_later(ProgressingOrder {
                order: ord.order,
                timestamp: ord.timestamp,
            })
            .await;
        assert!(!accepted)
    }

    #[tokio::test]
    async fn should_pop_best_order() {
        let mut backlog = setup_backlog(10, 5, 0);
        let ord1 = make_order(1, 1);
        let ord2 = make_order(2, 2);
        let ord3 = make_order(3, 3);
        backlog.put(ord1.into()).await;
        backlog.put(ord2.into()).await;
        backlog.put(ord3.clone().into()).await;

        let res = backlog.try_pop().await;
        assert_eq!(res, Some(ord3.order))
    }

    #[tokio::test]
    async fn should_always_pop_suspended_order_when_pa_100() {
        let mut backlog = setup_backlog(10, 5, 100);
        let ord1 = make_order(1, 1);
        let ord2 = make_order(2, 2);
        let ord3 = make_order(3, 3);
        backlog.put(ord1.into()).await;
        backlog.put(ord2.into()).await;
        backlog.put(ord3.clone().into()).await;
        let _ = backlog.try_pop().await;
        backlog.suspend(ord3.clone().order).await;

        let res = backlog.try_pop().await;
        assert_eq!(res, Some(ord3.order))
    }

    #[tokio::test]
    async fn should_not_pop_suspended_order_when_pa_0() {
        let mut backlog = setup_backlog(10, 5, 0);
        let ord1 = make_order(1, 1);
        let ord2 = make_order(2, 2);
        let ord3 = make_order(3, 3);
        backlog.put(ord1.into()).await;
        backlog.put(ord2.clone().into()).await;
        backlog.put(ord3.clone().into()).await;
        let _ = backlog.try_pop().await;
        backlog.suspend(ord3.clone().order).await;

        let res = backlog.try_pop().await;
        assert_eq!(res, Some(ord2.order))
    }

    #[tokio::test]
    async fn test_rocksdb_backlog() {
        fs::remove_dir_all("./tmp").unwrap();
        let mut store = RocksDBClient {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default("./tmp").unwrap()),
        };
        for i in 0..30 {
            store.put(make_order(i, i as u64)).await;
        }

        for i in 0..30 {
            assert!(<RocksDBClient as BacklogStore<MockOrder>>::exists(&store, MockOrderId(i)).await);
            assert_eq!(
                make_order(i, i as u64),
                <RocksDBClient as BacklogStore<MockOrder>>::get(&store, MockOrderId(i))
                    .await
                    .unwrap()
            );
        }

        for i in 0..30 {
            <RocksDBClient as BacklogStore<MockOrder>>::drop(&mut store, MockOrderId(i)).await;
            assert!(!<RocksDBClient as BacklogStore<MockOrder>>::exists(&store, MockOrderId(i)).await);
            assert!(
                <RocksDBClient as BacklogStore<MockOrder>>::get(&store, MockOrderId(i))
                    .await
                    .is_none()
            )
        }
    }
}
