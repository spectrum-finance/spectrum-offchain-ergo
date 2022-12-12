use async_trait::async_trait;
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

pub fn predicted_key_bytes<T: Serialize>(id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(PREDICTED_KEY_PREFIX).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

pub fn last_predicted_key_bytes<T: Serialize>(id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(LAST_PREDICTED_KEY_PREFIX).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

pub fn last_confirmed_key_bytes<T: Serialize>(id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(LAST_CONFIRMED_KEY_PREFIX).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

pub fn last_unconfirmed_key_bytes<T: Serialize>(id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(LAST_UNCONFIRMED_KEY_PREFIX).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

/// NOTE: do not run the tests in parallel! Rocksdb will complain about lock contention. Use the
/// following command: cargo test -- --test-threads=1
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ergo_chain_sync::cache::{redis::RedisClient, rocksdb::RocksDBClient};
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
    async fn test_redis_predicted() {
        let client = RedisClient::new("redis://127.0.0.1/");
        test_entity_repo_predicted(client).await;
    }

    #[tokio::test]
    async fn test_redis_confirmed() {
        let client = RedisClient::new("redis://127.0.0.1/");
        test_entity_repo_confirmed(client).await;
    }

    #[tokio::test]
    async fn test_redis_unconfirmed() {
        let client = RedisClient::new("redis://127.0.0.1/");
        test_entity_repo_unconfirmed(client).await;
    }

    #[tokio::test]
    async fn test_redis_invalidate() {
        let client = RedisClient::new("redis://127.0.0.1/");
        test_entity_repo_invalidate(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_predicted() {
        let client = rocks_db_client();
        test_entity_repo_predicted(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_confirmed() {
        let client = rocks_db_client();
        test_entity_repo_confirmed(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_unconfirmed() {
        let client = rocks_db_client();
        test_entity_repo_unconfirmed(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_invalidate() {
        let client = rocks_db_client();
        test_entity_repo_invalidate(client).await;
    }

    fn rocks_db_client() -> RocksDBClient {
        RocksDBClient {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default("./tmp").unwrap()),
        }
    }

    async fn test_entity_repo_predicted<C: EntityRepo<ErgoEntity>>(mut client: C) {
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

    async fn test_entity_repo_confirmed<C: EntityRepo<ErgoEntity>>(mut client: C) {
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

    async fn test_entity_repo_unconfirmed<C: EntityRepo<ErgoEntity>>(mut client: C) {
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

    async fn test_entity_repo_invalidate<C: EntityRepo<ErgoEntity>>(mut client: C) {
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
            <C as EntityRepo<ErgoEntity>>::invalidate(&mut client, token_ids[i], box_ids[i]).await;
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
