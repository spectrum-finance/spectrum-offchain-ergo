use async_trait::async_trait;
use ergo_chain_sync::cache::redis::RedisClient;
use redis::cmd;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::box_resolver::{Predicted, Traced};
use crate::data::unique_entity::{Confirmed, Unconfirmed};
use crate::data::OnChainEntity;

#[async_trait(?Send)]
pub trait EntityRepo<TEntity: OnChainEntity> {
    async fn get_prediction<'a>(&self, id: TEntity::TStateId) -> Option<Traced<Predicted<TEntity>>>
    where
        <TEntity as OnChainEntity>::TStateId: 'a;
    async fn get_last_predicted<'a>(&self, id: TEntity::TEntityId) -> Option<Predicted<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a;
    async fn get_last_confirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Confirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a;
    async fn get_last_unconfirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Unconfirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a;
    async fn put_predicted<'a>(&mut self, entity: Traced<Predicted<TEntity>>)
    where
        Traced<Predicted<TEntity>>: 'a;
    async fn put_confirmed<'a>(&mut self, entity: Confirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a;
    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a;
    async fn invalidate<'a>(&mut self, eid: TEntity::TEntityId, sid: TEntity::TStateId)
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
        <TEntity as OnChainEntity>::TEntityId: 'a;
}

static PREDICTED_KEY_PREFIX: &str = "predicted:prevState:";
static LAST_PREDICTED_KEY_PREFIX: &str = "predicted:last:";
static LAST_CONFIRMED_KEY_PREFIX: &str = "confirmed:last:";
static LAST_UNCONFIRMED_KEY_PREFIX: &str = "unconfirmed:last:";

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
}

fn predicted_key_bytes<T: Serialize>(id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(PREDICTED_KEY_PREFIX).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

fn last_predicted_key_bytes<T: Serialize>(id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(LAST_PREDICTED_KEY_PREFIX).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

fn last_confirmed_key_bytes<T: Serialize>(id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(LAST_CONFIRMED_KEY_PREFIX).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

fn last_unconfirmed_key_bytes<T: Serialize>(id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(LAST_UNCONFIRMED_KEY_PREFIX).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

#[cfg(test)]
mod tests {
    use ergo_lib::{
        ergo_chain_types::Digest32,
        ergotree_ir::chain::{ergo_box::BoxId, token::TokenId},
    };
    use serde::{Deserialize, Serialize};
    use sigma_test_util::force_any_val;

    use crate::{
        box_resolver::persistence::EntityRepo,
        data::{
            unique_entity::{Confirmed, Predicted, Traced, Unconfirmed},
            OnChainEntity,
        },
    };

    use super::RedisClient;

    #[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
    struct ErgoEntity {
        token_id: TokenId,
        box_id: BoxId,
    }
    impl OnChainEntity for ErgoEntity {
        type TEntityId = TokenId;

        type TStateId = BoxId;

        fn get_self_ref(&self) -> Self::TEntityId {
            self.token_id
        }

        fn get_self_state_ref(&self) -> Self::TStateId {
            self.box_id
        }
    }

    #[tokio::test]
    async fn test_entity_repo_predicted() {
        let mut client = RedisClient::new("redis://127.0.0.1/");
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        let mut entities = vec![];
        for i in 1..n {
            let entity = Traced {
                state: Predicted(ErgoEntity {
                    token_id: token_ids[i],
                    box_id: box_ids[i],
                }),
                prev_state_id: box_ids[i - 1],
            };
            client.put_predicted(entity.clone()).await;
            entities.push(entity);
        }
        for i in 1..n {
            let e: Traced<Predicted<ErgoEntity>> = client.get_prediction(box_ids[i]).await.unwrap();
            let predicted: Predicted<ErgoEntity> = client.get_last_predicted(token_ids[i]).await.unwrap();
            assert_eq!(e.state.0, entities[i - 1].state.0);
            assert_eq!(e.state.0, predicted.0);
        }
    }

    #[tokio::test]
    async fn test_entity_repo_confirmed() {
        let mut client = RedisClient::new("redis://127.0.0.1/");
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        let mut entities = vec![];
        for i in 0..n {
            let entity = Confirmed(ErgoEntity {
                token_id: token_ids[i],
                box_id: box_ids[i],
            });
            client.put_confirmed(entity.clone()).await;
            entities.push(entity);
        }
        for i in 0..n {
            let e: Confirmed<ErgoEntity> = client.get_last_confirmed(token_ids[i]).await.unwrap();
            assert_eq!(e.0, entities[i].0);
        }
    }

    #[tokio::test]
    async fn test_entity_repo_unconfirmed() {
        let mut client = RedisClient::new("redis://127.0.0.1/");
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        let mut entities = vec![];
        for i in 0..n {
            let entity = Unconfirmed(ErgoEntity {
                token_id: token_ids[i],
                box_id: box_ids[i],
            });
            client.put_unconfirmed(entity.clone()).await;
            entities.push(entity);
        }
        for i in 0..n {
            let e: Unconfirmed<ErgoEntity> = client.get_last_unconfirmed(token_ids[i]).await.unwrap();
            assert_eq!(e.0, entities[i].0);
        }
    }

    #[tokio::test]
    async fn test_entity_repo_invalidate() {
        let mut client = RedisClient::new("redis://127.0.0.1/");
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        for i in 1..n {
            let ee = ErgoEntity {
                token_id: token_ids[i],
                box_id: box_ids[i],
            };
            let entity = Traced {
                state: Predicted(ee.clone()),
                prev_state_id: box_ids[i - 1],
            };
            client.put_predicted(entity.clone()).await;
            client.put_unconfirmed(Unconfirmed(ee)).await;

            // Invalidate
            <RedisClient as EntityRepo<ErgoEntity>>::invalidate(&mut client, token_ids[i], box_ids[i]).await;
            let predicted: Option<Predicted<ErgoEntity>> = client.get_last_predicted(token_ids[i]).await;
            let unconfirmed: Option<Unconfirmed<ErgoEntity>> =
                client.get_last_unconfirmed(token_ids[i]).await;
            assert!(predicted.is_none());
            assert!(unconfirmed.is_none());
        }
    }

    fn gen_box_and_token_ids() -> (Vec<BoxId>, Vec<TokenId>, usize) {
        let box_ids: Vec<_> = force_any_val::<[Digest32; 30]>()
            .into_iter()
            .map(BoxId::from)
            .collect();
        let token_ids: Vec<_> = force_any_val::<[Digest32; 30]>()
            .into_iter()
            .map(|d| TokenId::from(BoxId::from(d)))
            .collect();
        (box_ids, token_ids, 30)
    }
}
