use std::fmt::Debug;
use std::sync::Arc;

use async_std::task::spawn_blocking;
use async_trait::async_trait;
use log::warn;
use serde::de::DeserializeOwned;
use serde::Serialize;

use ergo_chain_sync::rocksdb::RocksConfig;

use crate::binary::prefixed_key;
use crate::box_resolver::persistence::EntityRepo;
use crate::box_resolver::{Predicted, Traced};
use crate::data::unique_entity::{Confirmed, Unconfirmed};
use crate::data::OnChainEntity;

pub struct EntityRepoRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl EntityRepoRocksDB {
    pub fn new(conf: RocksConfig) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(conf.db_path).unwrap()),
        }
    }
}

const STATE_PREFIX: &str = "state";
const PREDICTION_LINK_PREFIX: &str = "prediction:link";
const LAST_PREDICTED_PREFIX: &str = "predicted:last";
const LAST_CONFIRMED_PREFIX: &str = "confirmed:last";
const LAST_UNCONFIRMED_PREFIX: &str = "unconfirmed:last";

#[async_trait(?Send)]
impl<TEntity> EntityRepo<TEntity> for EntityRepoRocksDB
where
    TEntity: OnChainEntity + Clone + Serialize + DeserializeOwned + Send + 'static,
    <TEntity as OnChainEntity>::TStateId: Clone + Serialize + DeserializeOwned + Send + Debug + 'static,
    <TEntity as OnChainEntity>::TEntityId: Clone + Serialize + DeserializeOwned + Send + 'static,
{
    async fn get_prediction_predecessor<'a>(
        &self,
        sid: <TEntity as OnChainEntity>::TStateId,
    ) -> Option<TEntity::TStateId>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        let db = self.db.clone();
        let link_key = prefixed_key(PREDICTION_LINK_PREFIX, &sid);
        spawn_blocking(move || {
            db.get(link_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
        })
        .await
    }

    async fn get_last_predicted<'a>(
        &self,
        id: <TEntity as OnChainEntity>::TEntityId,
    ) -> Option<Predicted<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        let db = self.db.clone();
        let index_key = prefixed_key(LAST_PREDICTED_PREFIX, &id);
        spawn_blocking(move || {
            db.get(index_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize::<'_, TEntity::TStateId>(&bytes).ok())
                .and_then(|sid| {
                    if db
                        .get(prefixed_key(PREDICTION_LINK_PREFIX, &sid))
                        .unwrap()
                        .is_some()
                    {
                        db.get(prefixed_key(STATE_PREFIX, &sid)).unwrap()
                    } else {
                        None
                    }
                })
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
                .map(Predicted)
        })
        .await
    }

    async fn get_last_confirmed<'a>(
        &self,
        id: <TEntity as OnChainEntity>::TEntityId,
    ) -> Option<Confirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        let db = self.db.clone();
        let index_key = prefixed_key(LAST_CONFIRMED_PREFIX, &id);
        spawn_blocking(move || {
            db.get(index_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize::<'_, TEntity::TStateId>(&bytes).ok())
                .and_then(|sid| db.get(prefixed_key(STATE_PREFIX, &sid)).unwrap())
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
                .map(Confirmed)
        })
        .await
    }

    async fn get_last_unconfirmed<'a>(
        &self,
        id: <TEntity as OnChainEntity>::TEntityId,
    ) -> Option<Unconfirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        let db = self.db.clone();
        let index_key = prefixed_key(LAST_UNCONFIRMED_PREFIX, &id);
        spawn_blocking(move || {
            db.get(index_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize::<'_, TEntity::TStateId>(&bytes).ok())
                .and_then(|sid| {
                    if db
                        .get(prefixed_key(LAST_UNCONFIRMED_PREFIX, &id))
                        .unwrap()
                        .is_some()
                    {
                        db.get(prefixed_key(STATE_PREFIX, &sid)).unwrap()
                    } else {
                        None
                    }
                })
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
                .map(Unconfirmed)
        })
        .await
    }

    async fn put_predicted<'a>(
        &mut self,
        Traced {
            state: Predicted(entity),
            prev_state_id,
        }: Traced<Predicted<TEntity>>,
    ) where
        Traced<Predicted<TEntity>>: 'a,
    {
        let db = self.db.clone();
        let state_id_bytes = bincode::serialize(&entity.get_self_state_ref()).unwrap();
        let state_key = prefixed_key(STATE_PREFIX, &entity.get_self_state_ref());
        let state_bytes = bincode::serialize(&entity).unwrap();
        let index_key = prefixed_key(LAST_PREDICTED_PREFIX, &entity.get_self_ref());
        let link_key = prefixed_key(PREDICTION_LINK_PREFIX, &entity.get_self_state_ref());
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.put(state_key, state_bytes).unwrap();
            tx.put(index_key, state_id_bytes).unwrap();
            if let Some(prev_sid) = prev_state_id {
                let prev_state_id_bytes = bincode::serialize(&prev_sid).unwrap();
                tx.put(link_key, prev_state_id_bytes).unwrap();
            }
            tx.commit().unwrap();
        })
        .await
    }

    async fn put_confirmed<'a>(&mut self, Confirmed(entity): Confirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let db = self.db.clone();
        let state_id_bytes = bincode::serialize(&entity.get_self_state_ref()).unwrap();
        let state_key = prefixed_key(STATE_PREFIX, &entity.get_self_state_ref());
        let state_bytes = bincode::serialize(&entity).unwrap();
        let index_key = prefixed_key(LAST_CONFIRMED_PREFIX, &entity.get_self_ref());
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.put(state_key, state_bytes).unwrap();
            tx.put(index_key, state_id_bytes).unwrap();
            tx.commit().unwrap();
        })
        .await
    }

    async fn put_unconfirmed<'a>(&mut self, Unconfirmed(entity): Unconfirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let db = self.db.clone();
        let state_id_bytes = bincode::serialize(&entity.get_self_state_ref()).unwrap();
        let state_key = prefixed_key(STATE_PREFIX, &entity.get_self_state_ref());
        let state_bytes = bincode::serialize(&entity).unwrap();
        let index_key = prefixed_key(LAST_UNCONFIRMED_PREFIX, &entity.get_self_ref());
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.put(state_key, state_bytes).unwrap();
            tx.put(index_key, state_id_bytes).unwrap();
            tx.commit().unwrap();
        })
        .await
    }

    async fn invalidate<'a>(
        &mut self,
        sid: <TEntity as OnChainEntity>::TStateId,
        eid: <TEntity as OnChainEntity>::TEntityId,
    ) where
        <TEntity as OnChainEntity>::TEntityId: 'a,
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        let predecessor: Option<<TEntity as OnChainEntity>::TStateId> =
            <EntityRepoRocksDB as EntityRepo<TEntity>>::get_prediction_predecessor::<'_, '_, '_>(
                self,
                sid.clone(),
            )
            .await;
        let db = self.db.clone();
        let link_key = prefixed_key(PREDICTION_LINK_PREFIX, &sid);
        let state_key = prefixed_key(STATE_PREFIX, &sid);
        let last_confirmed_index_key = prefixed_key(LAST_CONFIRMED_PREFIX, &eid);
        let last_predicted_index_key = prefixed_key(LAST_PREDICTED_PREFIX, &eid);
        let last_unconfirmed_index_key = prefixed_key(LAST_UNCONFIRMED_PREFIX, &eid);
        spawn_blocking(move || {
            let tx = db.transaction();
            if let Some(predecessor) = predecessor {
                warn!(target: "offchain_lm", "invalidate box: rollback to {:?}", predecessor);
                warn!("invalidate box: rollback to {:?}", predecessor);
                let predecessor_bytes = bincode::serialize(&predecessor).unwrap();
                tx.put(last_confirmed_index_key, predecessor_bytes).unwrap();
            } else {
                tx.delete(last_confirmed_index_key).unwrap();
            }
            tx.delete(link_key).unwrap();
            tx.delete(state_key).unwrap();
            tx.delete(last_unconfirmed_index_key).unwrap();
            tx.delete(last_predicted_index_key).unwrap();
            tx.commit().unwrap();
        })
        .await
    }

    async fn eliminate<'a>(&mut self, entity: TEntity)
    where
        TEntity: 'a,
    {
        let last_predicted_index_key = prefixed_key(LAST_PREDICTED_PREFIX, &entity.get_self_ref());
        let link_key = prefixed_key(PREDICTION_LINK_PREFIX, &entity.get_self_state_ref());
        let state_key = prefixed_key(STATE_PREFIX, &entity.get_self_state_ref());

        let last_confirmed_index_key = prefixed_key(LAST_CONFIRMED_PREFIX, &entity.get_self_ref());
        let last_unconfirmed_index_key = prefixed_key(LAST_UNCONFIRMED_PREFIX, &entity.get_self_ref());

        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.delete(link_key).unwrap();
            tx.delete(state_key).unwrap();
            tx.delete(last_predicted_index_key).unwrap();
            tx.delete(last_confirmed_index_key).unwrap();
            tx.delete(last_unconfirmed_index_key).unwrap();
            tx.commit().unwrap();
        })
        .await
    }

    async fn may_exist<'a>(&self, sid: <TEntity as OnChainEntity>::TStateId) -> bool
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        let db = self.db.clone();
        let state_key = prefixed_key(STATE_PREFIX, &sid);
        spawn_blocking(move || db.key_may_exist(state_key)).await
    }

    async fn get_state<'a>(&self, sid: <TEntity as OnChainEntity>::TStateId) -> Option<TEntity>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        let db = self.db.clone();
        let state_key = prefixed_key(STATE_PREFIX, &sid);
        spawn_blocking(move || {
            db.get(state_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
        })
        .await
    }
}
