use std::sync::Arc;

use async_trait::async_trait;
use rocksdb::{Direction, IteratorMode};
use tokio::task::spawn_blocking;

use spectrum_offchain::binary::prefixed_key;

use crate::data::PoolId;
use crate::scheduler::data::{PoolSchedule, Tick};

pub mod data;
pub mod process;

#[async_trait]
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

pub struct ScheduleRepoRocksDB {
    db: Arc<rocksdb::OptimisticTransactionDB>,
}

static TICK_PREFIX: &str = "tick";
static POOL_PREFIX: &str = "pool";

#[async_trait]
impl ScheduleRepo for ScheduleRepoRocksDB {
    async fn put_schedule(&mut self, schedule: PoolSchedule) {
        let db = Arc::clone(&self.db);
        let pid = schedule.pool_id;
        let ticks: Vec<Tick> = schedule.into();
        spawn_blocking(move || {
            let transaction = db.transaction();
            for tick in ticks {
                let key = prefixed_key(TICK_PREFIX, &tick.height);
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
        spawn_blocking(move || {
            let prefix = bincode::serialize(TICK_PREFIX).unwrap();
            if let Some(Ok((_, bytes))) = db
                .iterator(IteratorMode::From(&*prefix, Direction::Forward))
                .next()
            {
                bincode::deserialize(&bytes).ok()
            } else {
                None
            }
        })
        .await
        .unwrap()
    }

    async fn check_later(&mut self, tick: Tick) {
        todo!() // todo: DEV-601
    }

    async fn remove(&mut self, tick: Tick) {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            let key = prefixed_key(TICK_PREFIX, &tick.height);
            db.delete(key).unwrap()
        })
        .await
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ergo_lib::ergo_chain_types::Digest32;
    use ergo_lib::ergotree_ir::chain::token::TokenId;
    use itertools::Itertools;
    use rand::RngCore;

    use crate::data::PoolId;
    use crate::scheduler::data::{PoolSchedule, Tick};
    use crate::scheduler::{ScheduleRepo, ScheduleRepoRocksDB};

    fn rocks_db_client() -> ScheduleRepoRocksDB {
        let rnd = rand::thread_rng().next_u32();
        ScheduleRepoRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
        }
    }

    #[tokio::test]
    async fn put_schedue_peek_ticks() {
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
    async fn put_interfering_schedues_peek_ticks() {
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
}
