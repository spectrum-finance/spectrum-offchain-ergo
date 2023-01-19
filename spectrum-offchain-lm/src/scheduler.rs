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
        "boxId": "7153d14ec1fad42943102cc0541c96e44549f5c4e496e27f43b0885fd4a9fd43",
        "value": 1250000,
        "ergoTree": "19ec052404000400040204020404040404060406040804080404040204000400040204020400040a050005000404040204020e2074aeba0675c10c7fff46d3aa5e5a8efc55f0b0d87393dcb2f4b0a04be213cecb040004020500040204020406050005000402050205000500d81ed601b2a5730000d602db63087201d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b27203730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb27202730800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d6148c720902d615b27205730a00d6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205730f00d61b7e721a05d61c9d7206721bd61d998c720c028c720d02d61e998c720f028c721002d1ededededed93b272027310007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a70193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b172027311959172137312d802d61f9c721399721ba273137e721905d620b2a5731400ededed929a997206721472079c7e9995907219721a72199a721a7315731605721c937213f0721d93721ff0721eedededed93cbc272207317938602720e7213b2db630872207318009386027211721fb2db63087220731900e6c67220040893e4c67220050e8c720401958f7213731aededec929a997206721472079c7e9995907219721a72199a721a731b731c05721c92a39a9a72159c721a7217b27205731d0093721df0721392721e95917219721a731e9c721d99721ba2731f7e721905d801d61fe4c672010704edededed90721f9972197320909972149c7e99721a721f05721c9a721c7207907ef0998c7208027214069d9c7e721c067e721e067e997212732106937213732293721d7323",
        "assets": [
            {
                "tokenId": "48ad28d9bb55e1da36d27c655a84279ff25d889063255d3f774ff926a3704370",
                "amount": 1
            },
            {
                "tokenId": "0779ec04f2fae64e87418a1ad917639d4668f78484f45df962b0dec14a2591d2",
                "amount": 300000
            },
            {
                "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                "amount": 1
            },
            {
                "tokenId": "3fdce3da8d364f13bca60998c20660c79c19923f44e141df01349d2e63651e86",
                "amount": 100000000
            },
            {
                "tokenId": "c256908dd9fd477bde350be6a41c0884713a1b1d589357ae731763455ef28c10",
                "amount": 1500000000
            }
        ],
        "creationHeight": 921585,
        "additionalRegisters": {
            "R4": "100490031eaac170c801",
            "R5": "05becf24",
            "R6": "05d00f"
        },
        "transactionId": "ea34ff4653ce1579cb96464014ca072d1574fc04ac58f159786c3a1debebac2b",
        "index": 0
    }"#;
}
