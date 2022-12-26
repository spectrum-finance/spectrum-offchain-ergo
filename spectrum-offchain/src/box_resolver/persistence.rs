use async_trait::async_trait;

use crate::box_resolver::{Predicted, Traced};
use crate::data::unique_entity::{Confirmed, Unconfirmed};
use crate::data::OnChainEntity;

/// Stores on-chain entities.
/// Operations are atomic.
#[async_trait(?Send)]
pub trait EntityRepo<TEntity: OnChainEntity> {
    /// Get state id preceding given predicted state.
    async fn get_prediction_predecessor<'a>(&self, id: TEntity::TStateId) -> Option<TEntity::TStateId>
    where
        <TEntity as OnChainEntity>::TStateId: 'a;
    /// Get last predicted state of the given entity.
    async fn get_last_predicted<'a>(&self, id: TEntity::TEntityId) -> Option<Predicted<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a;
    /// Get last confirmed state of the given entity.
    async fn get_last_confirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Confirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a;
    /// Get last unconfirmed state of the given entity.
    async fn get_last_unconfirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Unconfirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a;
    /// Persist predicted state of the entity.
    async fn put_predicted<'a>(&mut self, entity: Traced<Predicted<TEntity>>)
    where
        Traced<Predicted<TEntity>>: 'a;
    /// Persist confirmed state of the entity.
    async fn put_confirmed<'a>(&mut self, entity: Confirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a;
    /// Persist unconfirmed state of the entity.
    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a;
    /// Invalidate particular state of the entity.
    async fn invalidate<'a>(&mut self, sid: TEntity::TStateId)
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
        <TEntity as OnChainEntity>::TEntityId: 'a;
    /// Invalidate particular state of the entity.
    async fn eliminate<'a>(&mut self, entity: TEntity)
    where
        TEntity: 'a;
    /// False-positive analog of `exists()`.
    async fn may_exist<'a>(&self, sid: TEntity::TStateId) -> bool
    where
        <TEntity as OnChainEntity>::TStateId: 'a;
    async fn get_state<'a>(&self, sid: TEntity::TStateId) -> Option<TEntity>
    where
        <TEntity as OnChainEntity>::TStateId: 'a;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ergo_lib::{
        ergo_chain_types::Digest32,
        ergotree_ir::chain::{ergo_box::BoxId, token::TokenId},
    };
    use rand::RngCore;
    use serde::{Deserialize, Serialize};
    use sigma_test_util::force_any_val;

    use crate::box_resolver::rocksdb::EntityRepoRocksDB;
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
    async fn test_rocksdb_may_exist() {
        let client = rocks_db_client();
        test_entity_repo_may_exist(client).await;
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

    #[tokio::test]
    async fn test_rocksdb_eliminate() {
        let client = rocks_db_client();
        test_entity_repo_eliminate(client).await;
    }

    fn rocks_db_client() -> EntityRepoRocksDB {
        let rnd = rand::thread_rng().next_u32();
        EntityRepoRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
        }
    }

    async fn test_entity_repo_may_exist<C: EntityRepo<ErgoEntity>>(mut client: C) {
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        for i in 1..n {
            let entity = Traced {
                state: Predicted(ErgoEntity {
                    token_id: token_ids[i],
                    box_id: box_ids[i],
                }),
                prev_state_id: box_ids.get(i - 1).cloned(),
            };
            client.put_predicted(entity.clone()).await;
        }
        for i in 1..n {
            let may_exist = client.may_exist(box_ids[i]).await;
            assert!(may_exist);
        }
    }

    async fn test_entity_repo_predicted<C: EntityRepo<ErgoEntity>>(mut client: C) {
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        for i in 1..n {
            let entity = Traced {
                state: Predicted(ErgoEntity {
                    token_id: token_ids[i],
                    box_id: box_ids[i],
                }),
                prev_state_id: box_ids.get(i - 1).cloned(),
            };
            client.put_predicted(entity.clone()).await;
        }
        for i in 1..n {
            let pred: Option<BoxId> = client.get_prediction_predecessor(box_ids[i]).await;
            assert_eq!(pred, box_ids.get(i - 1).cloned());
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
                prev_state_id: box_ids.get(i - 1).cloned(),
            };
            client.put_predicted(entity.clone()).await;
            client.put_unconfirmed(Unconfirmed(ee)).await;

            // Invalidate
            <C as EntityRepo<ErgoEntity>>::invalidate(&mut client, box_ids[i]).await;
            let predicted: Option<Predicted<ErgoEntity>> = client.get_last_predicted(token_ids[i]).await;
            let unconfirmed: Option<Unconfirmed<ErgoEntity>> =
                client.get_last_unconfirmed(token_ids[i]).await;
            assert!(predicted.is_none());
            assert!(unconfirmed.is_none());
        }
    }

    async fn test_entity_repo_eliminate<C: EntityRepo<ErgoEntity>>(mut client: C) {
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        for i in 1..n {
            let ee = ErgoEntity {
                token_id: token_ids[i],
                box_id: box_ids[i],
            };
            let entity = Traced {
                state: Predicted(ee.clone()),
                prev_state_id: box_ids.get(i - 1).cloned(),
            };
            client.put_predicted(entity.clone()).await;
            client.put_unconfirmed(Unconfirmed(ee.clone())).await;

            // Invalidate
            <C as EntityRepo<ErgoEntity>>::eliminate(&mut client, ee).await;
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
