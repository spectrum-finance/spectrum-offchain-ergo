use async_trait::async_trait;
use redis::cmd;
use serde::de::DeserializeOwned;
use serde::Serialize;

use ergo_chain_sync::cache::redis::RedisClient;

use crate::box_resolver::{Predicted, Traced};
use crate::data::unique_entity::{Confirmed, Unconfirmed};
use crate::data::OnChainEntity;

use super::persistence::{
    last_confirmed_key_bytes, last_predicted_key_bytes, last_unconfirmed_key_bytes, predicted_key_bytes,
    EntityRepo,
};

#[async_trait(?Send)]
impl<TEntity> EntityRepo<TEntity> for RedisClient
where
    TEntity: OnChainEntity + Clone + Serialize + DeserializeOwned,
    <TEntity as OnChainEntity>::TStateId: Clone + Serialize + DeserializeOwned,
    <TEntity as OnChainEntity>::TEntityId: Clone + Serialize + DeserializeOwned,
{
    async fn get_prediction<'a>(
        &self,
        id: <TEntity as OnChainEntity>::TStateId,
    ) -> Option<Traced<Predicted<TEntity>>>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        let mut conn = self.pool.get().await.unwrap();
        if let Ok(entity_bytes) = cmd("GET")
            .arg(predicted_key_bytes(&id))
            .query_async::<_, Vec<u8>>(&mut conn)
            .await
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
        let mut conn = self.pool.get().await.unwrap();
        if let Ok(entity_bytes) = cmd("GET")
            .arg(last_predicted_key_bytes(&id))
            .query_async::<_, Vec<u8>>(&mut conn)
            .await
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
        let mut conn = self.pool.get().await.unwrap();
        if let Ok(entity_bytes) = cmd("GET")
            .arg(last_confirmed_key_bytes(&id))
            .query_async::<_, Vec<u8>>(&mut conn)
            .await
        {
            match bincode::deserialize(&entity_bytes) {
                Ok(confirmed) => Some(confirmed),
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
        let mut conn = self.pool.get().await.unwrap();
        if let Ok(entity_bytes) = cmd("GET")
            .arg(last_unconfirmed_key_bytes(&id))
            .query_async::<_, Vec<u8>>(&mut conn)
            .await
        {
            match bincode::deserialize(&entity_bytes) {
                Ok(unconfirmed) => Some(unconfirmed),
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
        let mut conn = self.pool.get().await.unwrap();
        let mut pipe = redis::pipe();
        let _: () = pipe
            .atomic() // Start transaction
            .cmd("SET")
            .arg(last_predicted_key_bytes(&entity.state.get_self_ref()))
            .arg(bincode::serialize(&entity.state.0).unwrap())
            .ignore()
            .cmd("SET")
            .arg(predicted_key_bytes(&entity.state.get_self_state_ref()))
            .arg(bincode::serialize(&entity).unwrap())
            .query_async(&mut conn)
            .await
            .unwrap();
    }

    async fn put_confirmed<'a>(&mut self, entity: Confirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let mut conn = self.pool.get().await.unwrap();
        let _: () = cmd("SET")
            .arg(last_confirmed_key_bytes(&entity.0.get_self_ref()))
            .arg(bincode::serialize(&entity).unwrap())
            .query_async(&mut conn)
            .await
            .unwrap();
    }

    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let mut conn = self.pool.get().await.unwrap();
        let _: () = cmd("SET")
            .arg(last_unconfirmed_key_bytes(&entity.0.get_self_ref()))
            .arg(bincode::serialize(&entity).unwrap())
            .query_async(&mut conn)
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
        let mut conn = self.pool.get().await.unwrap();
        let mut pipe = redis::pipe();

        let mut first_cmd = false;
        let last_predicted_state: Option<Predicted<TEntity>> = self.get_last_predicted(eid.clone()).await;
        if let Some(last_predicted_state) = last_predicted_state {
            if last_predicted_state.get_self_state_ref() == sid {
                first_cmd = true;

                pipe.atomic() // Start transaction
                    .cmd("DEL")
                    .arg(last_predicted_key_bytes(&eid))
                    .ignore();
            }
        }

        let last_unconfirmed_state: Option<Unconfirmed<TEntity>> =
            self.get_last_unconfirmed(eid.clone()).await;
        if let Some(last_unconfirmed_state) = last_unconfirmed_state {
            if last_unconfirmed_state.0.get_self_state_ref() == sid {
                if !first_cmd {
                    pipe.atomic();
                }

                pipe.cmd("DEL").arg(last_unconfirmed_key_bytes(&eid)).ignore();
            }
        }

        let _: () = pipe
            .cmd("DEL")
            .arg(predicted_key_bytes(&sid))
            .query_async(&mut conn)
            .await
            .unwrap();
    }

    async fn get_state<'a>(&self, sid: <TEntity as OnChainEntity>::TStateId) -> Option<TEntity>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        todo!()
    }
}
