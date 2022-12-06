use async_trait::async_trait;

use crate::box_resolver::{Predicted, Traced};
use crate::data::unique_entity::{Confirmed, Unconfirmed};
use crate::data::OnChainEntity;

#[async_trait(?Send)]
pub trait EntityRepo<TEntity: OnChainEntity> {
    async fn get_prediction(&self, id: TEntity::TStateId) -> Option<Traced<Predicted<TEntity>>>;
    async fn get_last_predicted(&self, id: TEntity::TEntityId) -> Option<Predicted<TEntity>>;
    async fn get_last_confirmed(&self, id: TEntity::TEntityId) -> Option<Confirmed<TEntity>>;
    async fn get_last_unconfirmed(&self, id: TEntity::TEntityId) -> Option<Unconfirmed<TEntity>>;
    async fn put_predicted(&mut self, entity: Traced<Predicted<TEntity>>);
    async fn put_confirmed(&mut self, entity: Confirmed<TEntity>);
    async fn put_unconfirmed(&mut self, entity: Unconfirmed<TEntity>);
    async fn invalidate(&mut self, eid: TEntity::TEntityId, sid: TEntity::TStateId);
}
