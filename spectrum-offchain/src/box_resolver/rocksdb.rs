use async_trait::async_trait;
use ergo_chain_sync::cache::rocksdb::RocksDBClient;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::task::spawn_blocking;

use crate::box_resolver::{Predicted, Traced};
use crate::data::unique_entity::{Confirmed, Unconfirmed};
use crate::data::OnChainEntity;

use super::persistence::{
    last_confirmed_key_bytes, last_predicted_key_bytes, last_unconfirmed_key_bytes, predicted_key_bytes,
    EntityRepo,
};

#[async_trait(?Send)]
impl<TEntity> EntityRepo<TEntity> for RocksDBClient
where
    TEntity: OnChainEntity + Clone + Serialize + DeserializeOwned + Send + 'static,
    <TEntity as OnChainEntity>::TStateId: Clone + Serialize + DeserializeOwned + Send + 'static,
    <TEntity as OnChainEntity>::TEntityId: Clone + Serialize + DeserializeOwned + Send + 'static,
{
    async fn get_prediction<'a>(
        &self,
        id: <TEntity as OnChainEntity>::TStateId,
    ) -> Option<Traced<Predicted<TEntity>>>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        let db = self.db.clone();

        if let Some(entity_bytes) = spawn_blocking(move || db.get(&predicted_key_bytes(&id)).unwrap())
            .await
            .unwrap()
        {
            match bincode::deserialize(&entity_bytes) {
                Ok(predicted) => Some(predicted),
                Err(_) => None,
            }
        } else {
            None
        }
    }

    async fn get_last_predicted<'a>(
        &self,
        id: <TEntity as OnChainEntity>::TEntityId,
    ) -> Option<Predicted<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        let db = self.db.clone();

        if let Some(entity_bytes) = spawn_blocking(move || db.get(&last_predicted_key_bytes(&id)).unwrap())
            .await
            .unwrap()
        {
            match bincode::deserialize(&entity_bytes) {
                Ok(predicted) => Some(predicted),
                Err(_) => None,
            }
        } else {
            None
        }
    }

    async fn get_last_confirmed<'a>(
        &self,
        id: <TEntity as OnChainEntity>::TEntityId,
    ) -> Option<Confirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        let db = self.db.clone();

        if let Some(entity_bytes) = spawn_blocking(move || db.get(&last_confirmed_key_bytes(&id)).unwrap())
            .await
            .unwrap()
        {
            match bincode::deserialize(&entity_bytes) {
                Ok(predicted) => Some(predicted),
                Err(_) => None,
            }
        } else {
            None
        }
    }

    async fn get_last_unconfirmed<'a>(
        &self,
        id: <TEntity as OnChainEntity>::TEntityId,
    ) -> Option<Unconfirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        let db = self.db.clone();

        if let Some(entity_bytes) = spawn_blocking(move || db.get(&last_unconfirmed_key_bytes(&id)).unwrap())
            .await
            .unwrap()
        {
            match bincode::deserialize(&entity_bytes) {
                Ok(predicted) => Some(predicted),
                Err(_) => None,
            }
        } else {
            None
        }
    }

    async fn put_predicted<'a>(&mut self, entity: Traced<Predicted<TEntity>>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let db = self.db.clone();

        spawn_blocking(move || {
            let traced_entity_bytes = bincode::serialize(&entity).unwrap();
            let entity_id_bytes = last_predicted_key_bytes(&entity.state.get_self_ref());
            let entity_bytes = bincode::serialize(&entity.state.0).unwrap();
            let state_id_bytes = predicted_key_bytes(&entity.state.get_self_state_ref());

            let transaction = db.transaction();
            transaction.put(&entity_id_bytes, entity_bytes).unwrap();
            transaction.put(&state_id_bytes, &traced_entity_bytes).unwrap();

            transaction.commit().unwrap();
        })
        .await
        .unwrap();
    }

    async fn put_confirmed<'a>(&mut self, entity: Confirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let db = self.db.clone();

        spawn_blocking(move || {
            let entity_bytes = bincode::serialize(&entity).unwrap();
            let entity_id_bytes = last_confirmed_key_bytes(&entity.0.get_self_ref());
            db.put(&entity_id_bytes, entity_bytes).unwrap();
        })
        .await
        .unwrap();
    }

    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let db = self.db.clone();

        spawn_blocking(move || {
            let entity_bytes = bincode::serialize(&entity).unwrap();
            let entity_id_bytes = last_unconfirmed_key_bytes(&entity.0.get_self_ref());
            db.put(&entity_id_bytes, entity_bytes).unwrap();
        })
        .await
        .unwrap();
    }

    async fn invalidate<'a>(
        &mut self,
        eid: <TEntity as OnChainEntity>::TEntityId,
        sid: <TEntity as OnChainEntity>::TStateId,
    ) where
        <TEntity as OnChainEntity>::TEntityId: 'a,
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        let db = self.db.clone();

        let last_predicted_state: Option<Predicted<TEntity>> = self.get_last_predicted(eid.clone()).await;
        let delete_last_predicted_key = if let Some(last_predicted_state) = last_predicted_state {
            last_predicted_state.get_self_state_ref() == sid
        } else {
            false
        };
        let last_unconfirmed_state: Option<Unconfirmed<TEntity>> =
            self.get_last_unconfirmed(eid.clone()).await;
        let delete_last_unconfirmed_key = if let Some(last_unconfirmed_state) = last_unconfirmed_state {
            last_unconfirmed_state.0.get_self_state_ref() == sid
        } else {
            false
        };

        spawn_blocking(move || {
            let last_predicted_entity_id_bytes = last_predicted_key_bytes(&eid);
            let last_unconfirmed_entity_id_bytes = last_unconfirmed_key_bytes(&eid);
            let predicted_state_id_bytes = predicted_key_bytes(&sid);

            let transaction = db.transaction();

            if delete_last_predicted_key {
                transaction.delete(&last_predicted_entity_id_bytes).unwrap();
            }

            if delete_last_unconfirmed_key {
                transaction.delete(&last_unconfirmed_entity_id_bytes).unwrap();
            }

            transaction.delete(&predicted_state_id_bytes).unwrap();

            transaction.commit().unwrap();
        })
        .await
        .unwrap();
    }
}
