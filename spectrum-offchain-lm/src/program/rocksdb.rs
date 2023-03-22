use std::sync::Arc;

use async_std::task::spawn_blocking;
use async_trait::async_trait;

use ergo_chain_sync::rocksdb::RocksConfig;

use crate::data::pool::ProgramConfig;
use crate::data::PoolId;
use crate::program::ProgramRepo;

pub struct ProgramRepoRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl ProgramRepoRocksDB {
    pub fn new(conf: RocksConfig) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(conf.db_path).unwrap()),
        }
    }
}

#[async_trait]
impl ProgramRepo for ProgramRepoRocksDB {
    async fn put(&self, pool_id: PoolId, conf: ProgramConfig) {
        let db = self.db.clone();
        let key = bincode::serialize(&pool_id).unwrap();
        let value = bincode::serialize(&conf).unwrap();
        spawn_blocking(move || {
            db.put(key, value).unwrap();
        })
        .await
    }

    async fn get(&self, pool_id: PoolId) -> Option<ProgramConfig> {
        let db = self.db.clone();
        let key = bincode::serialize(&pool_id).unwrap();
        spawn_blocking(move || {
            db.get(&key)
                .unwrap()
                .and_then(|bs| bincode::deserialize(&*bs).ok())
        })
        .await
    }

    async fn exists(&self, pool_id: PoolId) -> bool {
        let db = self.db.clone();
        let key = bincode::serialize(&pool_id).unwrap();
        spawn_blocking(move || db.get(&key).unwrap().is_some()).await
    }
}
