use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::task::spawn_blocking;

use crate::backlog::data::BacklogOrder;
use crate::data::OnChainOrder;
use ergo_chain_sync::rocksdb::RocksConfig;

#[async_trait(?Send)]
pub trait BacklogStore<TOrd>
where
    TOrd: OnChainOrder,
{
    async fn put(&mut self, ord: BacklogOrder<TOrd>);
    async fn exists(&self, ord_id: TOrd::TOrderId) -> bool;
    async fn drop(&mut self, ord_id: TOrd::TOrderId);
    async fn get(&self, ord_id: TOrd::TOrderId) -> Option<BacklogOrder<TOrd>>;
}

pub struct BacklogStoreRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl BacklogStoreRocksDB {
    pub fn new(conf: RocksConfig) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(conf.db_path).unwrap()),
        }
    }
}

#[async_trait(?Send)]
impl<TOrd> BacklogStore<TOrd> for BacklogStoreRocksDB
where
    TOrd: OnChainOrder + Serialize + DeserializeOwned + Send + 'static,
    TOrd::TOrderId: Serialize + DeserializeOwned + Send,
{
    async fn put(&mut self, ord: BacklogOrder<TOrd>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.put(
                bincode::serialize(&ord.order.get_self_ref()).unwrap(),
                bincode::serialize(&ord).unwrap(),
            )
            .unwrap();
        })
        .await
        .unwrap();
    }
    async fn exists(&self, ord_id: TOrd::TOrderId) -> bool {
        let db = self.db.clone();
        spawn_blocking(move || db.get(bincode::serialize(&ord_id).unwrap()).unwrap().is_some())
            .await
            .unwrap()
    }

    async fn drop(&mut self, ord_id: TOrd::TOrderId) {
        let db = self.db.clone();
        spawn_blocking(move || db.delete(bincode::serialize(&ord_id).unwrap()).unwrap())
            .await
            .unwrap();
    }

    async fn get(&self, ord_id: TOrd::TOrderId) -> Option<BacklogOrder<TOrd>> {
        let db = self.db.clone();
        spawn_blocking(move || {
            db.get(bincode::serialize(&ord_id).unwrap())
                .unwrap()
                .map(|b| bincode::deserialize(&b).unwrap())
        })
        .await
        .unwrap()
    }
}
