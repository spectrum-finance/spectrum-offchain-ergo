use std::fmt::Debug;

use async_trait::async_trait;
use log::trace;

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
    async fn invalidate<'a>(&mut self, sid: TEntity::TStateId, eid: TEntity::TEntityId)
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

pub struct EntityRepoTracing<R> {
    inner: R,
}

impl<R> EntityRepoTracing<R> {
    pub fn wrap(repo: R) -> Self {
        Self { inner: repo }
    }
}

#[async_trait(?Send)]
impl<TEntity, R> EntityRepo<TEntity> for EntityRepoTracing<R>
where
    TEntity: OnChainEntity,
    TEntity::TEntityId: Debug + Copy,
    TEntity::TStateId: Debug + Copy,
    R: EntityRepo<TEntity>,
{
    async fn get_prediction_predecessor<'a>(&self, id: TEntity::TStateId) -> Option<TEntity::TStateId>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        trace!(target: "box_resolver", "get_prediction_predecessor({:?})", id);
        let res = self.inner.get_prediction_predecessor(id).await;
        trace!(target: "box_resolver", "get_prediction_predecessor({:?}) -> {:?}", id, res);
        res
    }

    async fn get_last_predicted<'a>(&self, id: TEntity::TEntityId) -> Option<Predicted<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        trace!(target: "box_resolver", "get_last_predicted({:?})", id);
        let res = self.inner.get_last_predicted(id).await;
        trace!(target: "box_resolver", "get_last_predicted({:?}) -> {:?}", id, res.as_ref().map(|_| "<Entity>"));
        res
    }

    async fn get_last_confirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Confirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        trace!(target: "box_resolver", "get_last_confirmed({:?})", id);
        let res = self.inner.get_last_confirmed(id).await;
        trace!(target: "box_resolver", "get_last_confirmed({:?}) -> {:?}", id, res.as_ref().map(|_| "<Entity>"));
        res
    }

    async fn get_last_unconfirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Unconfirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        trace!(target: "box_resolver", "get_last_unconfirmed({:?})", id);
        let res = self.inner.get_last_unconfirmed(id).await;
        trace!(target: "box_resolver", "get_last_unconfirmed({:?}) -> {:?}", id, res.as_ref().map(|_| "<Entity>"));
        res
    }

    async fn put_predicted<'a>(&mut self, entity: Traced<Predicted<TEntity>>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let show_entity = format!(
            "<Entity({:?}, {:?})>",
            entity.state.get_self_ref(),
            entity.state.get_self_state_ref()
        );
        trace!(target: "box_resolver", "put_predicted({})", show_entity);
        self.inner.put_predicted(entity).await;
        trace!(target: "box_resolver", "put_predicted({}) -> ()", show_entity);
    }

    async fn put_confirmed<'a>(&mut self, entity: Confirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let show_entity = format!(
            "<Entity({:?}, {:?})>",
            entity.0.get_self_ref(),
            entity.0.get_self_state_ref()
        );
        trace!(target: "box_resolver", "put_confirmed({})", show_entity);
        self.inner.put_confirmed(entity).await;
        trace!(target: "box_resolver", "put_confirmed({}) -> ()", show_entity);
    }

    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let show_entity = format!(
            "<Entity({:?}, {:?})>",
            entity.0.get_self_ref(),
            entity.0.get_self_state_ref()
        );
        trace!(target: "box_resolver", "put_unconfirmed({})", show_entity);
        self.inner.put_unconfirmed(entity).await;
        trace!(target: "box_resolver", "put_unconfirmed({}) -> ()", show_entity);
    }

    async fn invalidate<'a>(&mut self, sid: TEntity::TStateId, eid: TEntity::TEntityId)
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        trace!(target: "box_resolver", "invalidate({:?})", sid);
        self.inner.invalidate(sid, eid).await;
        trace!(target: "box_resolver", "invalidate({:?}) -> ()", sid);
    }

    async fn eliminate<'a>(&mut self, entity: TEntity)
    where
        TEntity: 'a,
    {
        let show_entity = format!(
            "<Entity({:?}, {:?})>",
            entity.get_self_ref(),
            entity.get_self_state_ref()
        );
        trace!(target: "box_resolver", "eliminate({})", show_entity);
        self.inner.eliminate(entity).await;
        trace!(target: "box_resolver", "eliminate({}) -> ()", show_entity);
    }

    async fn may_exist<'a>(&self, sid: TEntity::TStateId) -> bool
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        trace!(target: "box_resolver", "may_exist({:?})", sid);
        let res = self.inner.may_exist(sid).await;
        trace!(target: "box_resolver", "may_exist({:?}) -> {}", sid, res);
        res
    }

    async fn get_state<'a>(&self, sid: TEntity::TStateId) -> Option<TEntity>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        trace!(target: "box_resolver", "get_state({:?})", sid);
        let res = self.inner.get_state(sid).await;
        let show_entity = res
            .as_ref()
            .map(|e| format!("<Entity({:?}, {:?})>", e.get_self_ref(), e.get_self_state_ref()));
        trace!(target: "box_resolver", "get_state({:?}) -> {:?}", sid, show_entity);
        res
    }
}

#[cfg(test)]
pub(crate) mod tests {
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
    pub struct ErgoEntity {
        pub token_id: TokenId,
        pub box_id: BoxId,
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

    pub fn rocks_db_client() -> EntityRepoRocksDB {
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
            client.put_unconfirmed(Unconfirmed(ee.clone())).await;
            client.put_confirmed(Confirmed(ee)).await;

            // Invalidate
            <C as EntityRepo<ErgoEntity>>::invalidate(&mut client, box_ids[i], token_ids[i]).await;
            let predicted: Option<Predicted<ErgoEntity>> = client.get_last_predicted(token_ids[i]).await;
            let unconfirmed: Option<Unconfirmed<ErgoEntity>> =
                client.get_last_unconfirmed(token_ids[i]).await;
            let confirmed: Option<Confirmed<ErgoEntity>> = client.get_last_confirmed(token_ids[i]).await;
            assert!(predicted.is_none());
            assert!(unconfirmed.is_none());
            assert!(confirmed.is_none());
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
