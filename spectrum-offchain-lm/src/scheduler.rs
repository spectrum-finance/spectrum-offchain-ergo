use std::sync::Arc;

use async_trait::async_trait;
use log::trace;
use rocksdb::{Direction, IteratorMode};
use tokio::task::spawn_blocking;

use ergo_chain_sync::rocksdb::RocksConfig;
use spectrum_offchain::binary::{prefixed_key, raw_prefixed_key};

use crate::data::PoolId;
use crate::scheduler::data::{PoolSchedule, Tick};

pub mod data;
pub mod process;

#[async_trait(?Send)]
pub trait ScheduleRepo {
    /// Persist schedule.
    async fn put_schedule(&mut self, schedule: PoolSchedule);
    /// Check whether a schedule for the given pool exists.
    async fn exists(&self, pool_id: PoolId) -> bool;
    /// Get closest tick.
    async fn peek(&mut self) -> Option<Tick>;
    /// Mark this tick as temporarily processed.
    async fn check_later(&mut self, tick: Tick);
    /// Remove tick from storage.
    async fn remove(&mut self, tick: Tick);
}

pub struct ScheduleRepoTracing<R> {
    inner: R,
}

impl<B> ScheduleRepoTracing<B> {
    pub fn wrap(backlog: B) -> Self {
        Self { inner: backlog }
    }
}

#[async_trait(?Send)]
impl<R> ScheduleRepo for ScheduleRepoTracing<R>
where
    R: ScheduleRepo,
{
    async fn put_schedule(&mut self, schedule: PoolSchedule) {
        trace!(target: "schedules", "put_schedule({})", schedule);
        self.inner.put_schedule(schedule.clone()).await;
        trace!(target: "schedules", "put_schedule({}) -> ()", schedule);
    }

    async fn exists(&self, pool_id: PoolId) -> bool {
        self.inner.exists(pool_id).await
    }

    async fn peek(&mut self) -> Option<Tick> {
        trace!(target: "schedules", "peek()");
        let res = self.inner.peek().await;
        trace!(target: "schedules", "peek() -> {:?}", res);
        res
    }

    async fn check_later(&mut self, tick: Tick) {
        trace!(target: "schedules", "check_later({:?})", tick);
        self.inner.check_later(tick.clone()).await;
        trace!(target: "schedules", "check_later({:?}) -> ()", tick);
    }

    async fn remove(&mut self, tick: Tick) {
        trace!(target: "schedules", "remove({:?})", tick);
        self.inner.remove(tick.clone()).await;
        trace!(target: "schedules", "remove({:?}) -> ()", tick);
    }
}

pub struct ScheduleRepoRocksDB {
    db: Arc<rocksdb::OptimisticTransactionDB>,
    /// Represents the index of `check_later` `Ticks` to next inspect. Don't need to persist this
    /// because it wouldn't hurt to start again at index 0.
    check_later_next_ix: usize,
}

impl ScheduleRepoRocksDB {
    pub fn new(conf: RocksConfig) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(conf.db_path).unwrap()),
            check_later_next_ix: 0,
        }
    }
}

#[async_trait(?Send)]
impl ScheduleRepo for ScheduleRepoRocksDB {
    async fn put_schedule(&mut self, schedule: PoolSchedule) {
        let db = Arc::clone(&self.db);
        let pid = schedule.pool_id;
        let ticks: Vec<Tick> = schedule.into();
        spawn_blocking(move || {
            let transaction = db.transaction();
            for tick in ticks {
                let key = tick_key(TICK_PREFIX, &tick);
                let value = bincode::serialize(&tick).unwrap();
                transaction.put(key, value).unwrap();
            }
            let pool_key = prefixed_key(POOL_PREFIX, &pid);
            transaction.put(pool_key, vec![0u8]).unwrap();
            transaction.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn exists(&self, pool_id: PoolId) -> bool {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || db.get(prefixed_key(POOL_PREFIX, &pool_id)).unwrap().is_some())
            .await
            .unwrap()
    }

    async fn peek(&mut self) -> Option<Tick> {
        let db = Arc::clone(&self.db);
        let ix = self.check_later_next_ix;
        let (tick, next_ix) = spawn_blocking(move || {
            let check_later_prefix = bincode::serialize(CHECK_LATER_PREFIX).unwrap();
            let ticks_to_check_later: Vec<Tick> = db
                .iterator(IteratorMode::From(&check_later_prefix, Direction::Forward))
                .flatten()
                .map(|(_, bytes)| bincode::deserialize(&bytes).unwrap())
                .collect();

            // We first try to find a `Tick` that has yet to be previously chosen.
            let prefix = bincode::serialize(TICK_PREFIX).unwrap();
            for (_, bytes) in db
                .iterator(IteratorMode::From(&prefix, Direction::Forward))
                .flatten()
            {
                let tick: Tick = bincode::deserialize(&bytes).unwrap();
                if !ticks_to_check_later.contains(&tick) {
                    return (Some(tick), ix);
                }
            }

            // Select a previously-chosen `Tick`.
            if !ticks_to_check_later.is_empty() {
                let next_ix = if ix + 1 == ticks_to_check_later.len() {
                    0
                } else {
                    ix + 1
                };
                return (Some(ticks_to_check_later[ix]), next_ix);
            }

            (None, 0)
        })
        .await
        .unwrap();

        self.check_later_next_ix = next_ix;
        tick
    }

    async fn check_later(&mut self, tick: Tick) {
        let db = Arc::clone(&self.db);
        let key = tick_key(CHECK_LATER_PREFIX, &tick);
        spawn_blocking(move || {
            db.put(key, bincode::serialize(&tick).unwrap()).unwrap();
        })
        .await
        .unwrap()
    }

    async fn remove(&mut self, tick: Tick) {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            let key = tick_key(TICK_PREFIX, &tick);
            db.delete(key).unwrap()
        })
        .await
        .unwrap()
    }
}

static CHECK_LATER_PREFIX: &str = "cl:tick";
static TICK_PREFIX: &str = "tick";
static POOL_PREFIX: &str = "pool";

fn tick_key(prefix: &str, tick: &Tick) -> Vec<u8> {
    let mut key_body = bincode::serialize(&tick.height).unwrap();
    key_body.reverse();
    raw_prefixed_key(prefix, key_body)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ergo_lib::ergo_chain_types::Digest32;
    use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
    use ergo_lib::ergotree_ir::chain::token::TokenId;
    use itertools::Itertools;
    use rand::RngCore;

    use spectrum_offchain::event_sink::handlers::types::TryFromBox;

    use crate::data::pool::Pool;
    use crate::data::{AsBox, PoolId};
    use crate::scheduler::data::{PoolSchedule, Tick};
    use crate::scheduler::{ScheduleRepo, ScheduleRepoRocksDB};

    fn rocks_db_client() -> ScheduleRepoRocksDB {
        let rnd = rand::thread_rng().next_u32();
        ScheduleRepoRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
            check_later_next_ix: 0,
        }
    }

    #[tokio::test]
    async fn put_schedule_peek_ticks() {
        let mut client = rocks_db_client();
        let schedule = PoolSchedule {
            pool_id: PoolId::from(TokenId::from(Digest32::zero())),
            ticks: vec![(1, 10), (2, 20), (3, 30)],
        };
        client.put_schedule(schedule.clone()).await;
        let mut ticks = Vec::new();
        while let Some(tick) = client.peek().await {
            ticks.push(tick);
            client.remove(tick).await;
        }
        assert_eq!(ticks, <Vec<Tick>>::from(schedule))
    }

    #[tokio::test]
    async fn put_real_schedule_peek_ticks() {
        let mut client = rocks_db_client();
        let pool_box: ErgoBox = serde_json::from_str(POOL_JSON).unwrap();
        let pool = <AsBox<Pool>>::try_from_box(pool_box).unwrap();
        let schedule = PoolSchedule::from(pool.1);
        client.put_schedule(schedule.clone()).await;
        let mut ticks = Vec::new();
        while let Some(tick) = client.peek().await {
            ticks.push(tick);
            client.remove(tick).await;
        }
        assert_eq!(ticks, <Vec<Tick>>::from(schedule))
    }

    #[tokio::test]
    async fn schedule_check_later() {
        let mut client = rocks_db_client();
        let pool_id = PoolId::from(TokenId::from(Digest32::zero()));
        let schedule = PoolSchedule {
            pool_id,
            ticks: vec![(1, 10), (2, 20), (3, 30)],
        };
        client.put_schedule(schedule.clone()).await;

        let tick1 = Tick {
            pool_id,
            epoch_ix: 1,
            height: 10,
        };
        let tick2 = Tick {
            pool_id,
            epoch_ix: 2,
            height: 20,
        };
        let tick3 = Tick {
            pool_id,
            epoch_ix: 3,
            height: 30,
        };

        client.check_later(tick1).await;
        let next_tick = client.peek().await.unwrap();
        assert_eq!(next_tick, tick2);

        // Try again, expect same result
        let next_tick = client.peek().await.unwrap();
        assert_eq!(next_tick, tick2);

        client.check_later(tick2).await;
        let next_tick = client.peek().await.unwrap();
        assert_eq!(next_tick, tick3);

        client.check_later(tick3).await;
        let next_tick = client.peek().await.unwrap();
        // This is the first time cycling through `check_later` Ticks.
        assert_eq!(next_tick, tick1);
    }

    #[tokio::test]
    async fn put_interfering_schedules_peek_ticks() {
        let mut client = rocks_db_client();
        let schedule_1 = PoolSchedule {
            pool_id: PoolId::from(TokenId::from(Digest32::zero())),
            ticks: vec![(1, 10), (2, 20), (3, 30)],
        };
        let schedule_2 = PoolSchedule {
            pool_id: PoolId::from(TokenId::from(Digest32::zero())),
            ticks: vec![(1, 5), (2, 15), (3, 25), (4, 35)],
        };
        client.put_schedule(schedule_1.clone()).await;
        client.put_schedule(schedule_2.clone()).await;
        let mut ticks = Vec::new();
        while let Some(tick) = client.peek().await {
            ticks.push(tick);
            client.remove(tick).await;
        }
        let sorted_ticks = <Vec<Tick> as From<PoolSchedule>>::from(schedule_1)
            .into_iter()
            .chain(<Vec<Tick> as From<PoolSchedule>>::from(schedule_2))
            .sorted_by_key(|t| t.height)
            .collect::<Vec<_>>();
        assert_eq!(ticks, sorted_ticks);
    }

    const POOL_JSON: &str = r#"{
        "boxId": "6a928dad5999caeaf08396f9a40c37b1b09ce2b972df457f4afacb61ff69009a",
        "value": 1250000,
        "ergoTree": "19e9041f04000402040204040404040604060408040804040402040004000402040204000400040a0500040204020500050004020402040605000500040205000500d81bd601b2a5730000d602db63087201d603db6308a7d604e4c6a70410d605e4c6a70505d606e4c6a70605d607b27202730100d608b27203730200d609b27202730300d60ab27203730400d60bb27202730500d60cb27203730600d60db27202730700d60eb27203730800d60f8c720a02d610998c720902720fd6118c720802d612b27204730900d6139a99a37212730ad614b27204730b00d6159d72137214d61695919e72137214730c9a7215730d7215d617b27204730e00d6187e721705d6199d72057218d61a998c720b028c720c02d61b998c720d028c720e02d1ededededed93b27202730f00b27203731000ededed93e4c672010410720493e4c672010505720593e4c6720106057206928cc77201018cc7a70193c27201c2a7ededed938c7207018c720801938c7209018c720a01938c720b018c720c01938c720d018c720e0193b172027311959172107312eded929a997205721172069c7e9995907216721772169a721773137314057219937210f0721a939c7210997218a273157e721605f0721b958f72107316ededec929a997205721172069c7e9995907216721772169a72177317731805721992a39a9a72129c72177214b2720473190093721af0721092721b959172167217731a9c721a997218a2731b7e721605d801d61ce4c672010704edededed90721c997216731c909972119c7e997217721c0572199a7219720693f0998c72070272117d9d9c7e7219067e721b067e720f0605937210731d93721a731e",
        "assets": [
            {
                "tokenId": "5c7b7988f34bfb13059c447c648c23c36d121228588fa9ea68b9943b0333ea4c",
                "amount": 1
            },
            {
                "tokenId": "0779ec04f2fae64e87418a1ad917639d4668f78484f45df962b0dec14a2591d2",
                "amount": 10000
            },
            {
                "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                "amount": 100
            },
            {
                "tokenId": "3fdce3da8d364f13bca60998c20660c79c19923f44e141df01349d2e63651e86",
                "amount": 10000000
            },
            {
                "tokenId": "c256908dd9fd477bde350be6a41c0884713a1b1d589357ae731763455ef28c10",
                "amount": 99999000
            }
        ],
        "creationHeight": 913825,
        "additionalRegisters": {
            "R4": "1004d00f1492d66fd00f",
            "R5": "05a09c01",
            "R6": "05d00f"
        },
        "transactionId": "ceb08428111f9d4dbcba3b0de024326af032e52f37ea189b8bd3affbcc40a3a6",
        "index": 0
    }"#;
}
