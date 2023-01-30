use std::sync::Arc;

use async_std::task::spawn_blocking;
use async_trait::async_trait;
use chrono::Utc;
use log::trace;
use rocksdb::{Direction, IteratorMode};

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
    /// Get closest tick.
    async fn peek(&mut self) -> Option<Tick>;
    /// Remove tried tick from db.
    async fn remove(&mut self, tick: Tick);
    /// Defer tick processing until the given timestamp.
    async fn defer(&mut self, tick: Tick, until: i64);
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

    async fn peek(&mut self) -> Option<Tick> {
        trace!(target: "schedules", "peek()");
        let res = self.inner.peek().await;
        trace!(target: "schedules", "peek() -> {:?}", res);
        res
    }

    async fn remove(&mut self, tick: Tick) {
        trace!(target: "schedules", "remove({:?})", tick);
        self.inner.remove(tick.clone()).await;
        trace!(target: "schedules", "remove({:?}) -> ()", tick);
    }

    async fn defer(&mut self, tick: Tick, until: i64) {
        trace!(target: "schedules", "defer(tick: {:?}, until: {})", tick, until);
        self.inner.defer(tick.clone(), until).await;
        trace!(target: "schedules", "defer(tick: {:?}, until: {})", tick, until);
    }
}

pub struct ScheduleRepoRocksDB {
    db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl ScheduleRepoRocksDB {
    pub fn new(conf: RocksConfig) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(conf.db_path).unwrap()),
        }
    }
}

#[async_trait(?Send)]
impl ScheduleRepo for ScheduleRepoRocksDB {
    async fn put_schedule(&mut self, schedule: PoolSchedule) {
        let db = Arc::clone(&self.db);
        let pid = schedule.pool_id;
        spawn_blocking(move || {
            if let Some(next_height) = schedule.next_compounding_at() {
                let transaction = db.transaction();
                let tried_tick = tick_key(DEFERRED_TICKS_PREFIX, &pid, &next_height);
                let tick = tick_key(TICKS_PREFIX, &pid, &next_height);
                // Write updated schedule only in case we haven't tried to compound it already on this height.
                if transaction.get(tried_tick).unwrap().is_none()
                    && transaction.get(tick.clone()).unwrap().is_none()
                {
                    let schedule_key = prefixed_key(SCHEDULE_PREFIX, &pid);
                    let schedule_bytes = bincode::serialize(&schedule).unwrap();
                    transaction.put(schedule_key, schedule_bytes).unwrap();
                    transaction.put(tick, vec![]).unwrap();
                }
                transaction.commit().unwrap();
            }
        })
        .await
    }

    async fn peek(&mut self) -> Option<Tick> {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            let ticks_prefix = bincode::serialize(TICKS_PREFIX).unwrap();
            let mut ticks = db.iterator(IteratorMode::From(&ticks_prefix, Direction::Forward));
            let mut tick: Option<Tick> = None;
            // First we try to peek closest pending tick.
            while tick.is_none() {
                if let Some((bs, _)) = ticks.next().and_then(|res| res.ok()) {
                    tick = destructure_tick_key(&*bs)
                        .and_then(|pid| {
                            let schedule_key = prefixed_key(SCHEDULE_PREFIX, &pid);
                            db.get(schedule_key).unwrap()
                        })
                        .and_then(|bs| bincode::deserialize::<PoolSchedule>(&bs).ok())
                        .and_then(|sc| sc.try_into().ok());
                } else {
                    break;
                }
            }
            // If there are no pending ticks we check deferred ticks.
            if tick.is_none() {
                let deferred_ticks_prefix = bincode::serialize(DEFERRED_TICKS_PREFIX).unwrap();
                let mut deferred_ticks =
                    db.iterator(IteratorMode::From(&deferred_ticks_prefix, Direction::Forward));
                let ts_now = Utc::now().timestamp();
                while tick.is_none() {
                    if let Some((bs, deferred_until)) = deferred_ticks.next().and_then(|res| res.ok()) {
                        if let Ok(deferred_until) = bincode::deserialize::<i64>(&deferred_until) {
                            if deferred_until <= ts_now {
                                tick = destructure_deferred_tick_key(&*bs)
                                    .and_then(|pid| {
                                        let schedule_key = prefixed_key(SCHEDULE_PREFIX, &pid);
                                        db.get(schedule_key).unwrap()
                                    })
                                    .and_then(|bs| bincode::deserialize::<PoolSchedule>(&bs).ok())
                                    .and_then(|sc| sc.try_into().ok());
                            } else {
                                break;
                            }
                        }
                    } else {
                        break;
                    }
                }
            }
            tick
        })
        .await
    }

    async fn remove(&mut self, tick: Tick) {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            let index = tick_key(DEFERRED_TICKS_PREFIX, &tick.pool_id, &tick.height);
            db.delete(index).unwrap()
        })
        .await
    }

    async fn defer(&mut self, tick: Tick, until: i64) {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            let transaction = db.transaction();
            let tried_tick_key = tick_key(DEFERRED_TICKS_PREFIX, &tick.pool_id, &tick.height);
            let until_bytes = bincode::serialize(&until).unwrap();
            let tick_key = tick_key(TICKS_PREFIX, &tick.pool_id, &tick.height);
            transaction.delete(tick_key).unwrap();
            transaction.put(tried_tick_key, until_bytes).unwrap();
            transaction.commit().unwrap();
        })
        .await
    }
}

/// Schedules: (PREFIX:PoolId -> Schedule)
const SCHEDULE_PREFIX: &str = "sc:";
/// Pending ticks (queue): (PREFIX:H:PoolId -> ())
const TICKS_PREFIX: &str = "ts:";
/// Tried ticks (queue): (PREFIX:H:PoolId -> RETRY_AFTER)
const DEFERRED_TICKS_PREFIX: &str = "tr:ts:";
const TICK_KEY_LEN: usize = 47;
const TRIED_TICK_KEY_LEN: usize = 50;

fn tick_key(pfx: &str, pool_id: &PoolId, next_epoch_height: &u32) -> Vec<u8> {
    let mut key_body = bincode::serialize(next_epoch_height).unwrap();
    key_body.reverse();
    let pool_id = bincode::serialize(pool_id).unwrap();
    key_body.extend_from_slice(&*pool_id);
    raw_prefixed_key(pfx, &key_body)
}

fn destructure_tick_key(bytes: &[u8]) -> Option<PoolId> {
    if bytes.len() == TICK_KEY_LEN {
        bincode::deserialize(&bytes[15..]).ok()
    } else {
        None
    }
}

fn destructure_deferred_tick_key(bytes: &[u8]) -> Option<PoolId> {
    if bytes.len() == TRIED_TICK_KEY_LEN {
        bincode::deserialize(&bytes[18..]).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ergo_lib::ergo_chain_types::Digest32;
    use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
    use ergo_lib::ergotree_ir::chain::token::TokenId;
    use rand::RngCore;
    use sigma_test_util::force_any_val;

    use spectrum_offchain::event_sink::handlers::types::TryFromBox;

    use crate::data::pool::Pool;
    use crate::data::{AsBox, PoolId};
    use crate::scheduler::data::PoolSchedule;
    use crate::scheduler::{
        destructure_deferred_tick_key, destructure_tick_key, tick_key, ScheduleRepo, ScheduleRepoRocksDB,
        DEFERRED_TICKS_PREFIX, TICKS_PREFIX,
    };

    fn rocks_db_client() -> ScheduleRepoRocksDB {
        let rnd = rand::thread_rng().next_u32();
        ScheduleRepoRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
        }
    }

    #[test]
    fn tick_index_serialization_roundtrip() {
        let pool_id = PoolId::from(force_any_val::<TokenId>());
        let height = 10000u32;
        let index = tick_key(TICKS_PREFIX, &pool_id, &height);
        let pool_id_extracted = destructure_tick_key(&index);
        assert_eq!(pool_id_extracted, Some(pool_id));
    }

    #[test]
    fn tried_tick_index_serialization_roundtrip() {
        let pool_id = PoolId::from(force_any_val::<TokenId>());
        let height = 10000u32;
        let index = tick_key(DEFERRED_TICKS_PREFIX, &pool_id, &height);
        let pool_id_extracted = destructure_deferred_tick_key(&index);
        assert_eq!(pool_id_extracted, Some(pool_id));
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
            client.defer(tick, <i64>::MAX).await;
        }
        assert_eq!(ticks[0].height, schedule.next_compounding_at().unwrap())
    }

    #[tokio::test]
    async fn put_interfering_schedules_peek_ticks() {
        let mut client = rocks_db_client();
        let schedule_1 = PoolSchedule {
            pool_id: PoolId::from(TokenId::from(Digest32::from([0u8; 32]))),
            epoch_len: 10,
            epoch_num: 10,
            program_start: 100,
            last_completed_epoch_ix: 0,
        };
        let schedule_2 = PoolSchedule {
            pool_id: PoolId::from(TokenId::from(Digest32::from([1u8; 32]))),
            epoch_len: 15,
            epoch_num: 15,
            program_start: 110,
            last_completed_epoch_ix: 0,
        };
        client.put_schedule(schedule_1.clone()).await;
        client.put_schedule(schedule_2.clone()).await;
        let mut ticks = Vec::new();
        while let Some(tick) = client.peek().await {
            ticks.push(tick);
            client.defer(tick, <i64>::MAX).await;
        }
        assert_eq!(ticks[0].height, schedule_1.next_compounding_at().unwrap());
        assert_eq!(ticks[1].height, schedule_2.next_compounding_at().unwrap());
    }

    #[tokio::test]
    async fn peek_defer_remove() {
        let mut client = rocks_db_client();
        let schedule_1 = PoolSchedule {
            pool_id: PoolId::from(TokenId::from(Digest32::from([0u8; 32]))),
            epoch_len: 10,
            epoch_num: 10,
            program_start: 100,
            last_completed_epoch_ix: 0,
        };
        client.put_schedule(schedule_1.clone()).await;
        let tick = client.peek().await;
        assert!(tick.is_some());
        client.defer(tick.unwrap(), 0).await;
        let deferred_tick = client.peek().await;
        assert!(deferred_tick.is_some());
        assert_eq!(deferred_tick, tick);
        client.remove(deferred_tick.unwrap()).await;
        assert!(client.peek().await.is_none());
    }

    const POOL_JSON: &str = r#"{
        "boxId": "6c70330b8bfe30b5b3ae8b22be6bccf850cda24c3e4cd057d6fea12df3c694bb",
        "value": 1250000,
        "ergoTree": "199d062704000400040204020404040404060406040804080404040204000400040204020601010400040a0500050004040e20a20a53f905f41ebdd71c2c239f270392d0ae0f23f6bd9f3687d166eea745bbf60400040205000402040204060500050005feffffffffffffffff010502050005000402050005000100d820d601b2a5730000d602db63087201d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b27203730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb27202730800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d614998c720c028c720d02d615b27205730a00d6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205730f00d61b7e721a05d61c8c721002d61d998c720f02721cd61e8c720902d61f9d7206721bd6207310d1ededededed93b272027311007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a70193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b172027312959172137313d802d6219c721399721ba273147e721905d622b2a5731500eded937213f07214937221f0721dedededed93cbc272227316938602720e7213b2db6308722273170093860272117221b2db63087222731800e6c67222040893e4c67222050e8c720401958f72137319ededec929a997206721e72079c7e9995907219721a72199a721a731a731b05721f92a39a9a72159c721a7217b27205731c00937214f0721392721d95917219721a731d9c721499721ba2731e7e721905d804d621e4c672010704d62299721a7221d6237e722205d6249999731f721c9c9972127320722395ed917224732191721d7322edededed90722199721973239099721e9c7223721f9a721f7207907ef0998c720802721e069a9d9c99997e721e069d9c7e7206067e7222067e721a0672207e721d067e7224067220937213732493721473257326",
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
