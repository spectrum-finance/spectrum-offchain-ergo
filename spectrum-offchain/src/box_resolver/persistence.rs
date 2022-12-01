use async_trait::async_trait;

use crate::box_resolver::{Predicted, Traced};
use crate::data::unique_entity::{Confirmed, Unconfirmed};

#[async_trait(?Send)]
pub trait EntityRepo<TEntity, TEntityId, TStateId> {
    async fn get_prediction(&self, id: TStateId) -> Option<Traced<Predicted<TEntity>, TStateId>>;
    async fn get_last_predicted(&self, id: TEntityId) -> Option<Predicted<TEntity>>;
    async fn get_last_confirmed(&self, id: TEntityId) -> Option<Confirmed<TEntity>>;
    async fn get_last_unconfirmed(&self, id: TEntityId) -> Option<Unconfirmed<TEntity>>;
    async fn put_predicted(&mut self, entity: Traced<Predicted<TEntity>, TStateId>);
    async fn put_confirmed(&mut self, entity: Confirmed<TEntity>);
    async fn put_unconfirmed(&mut self, entity: Unconfirmed<TEntity>);
    async fn invalidate(&mut self, eid: TEntityId, sid: TStateId);
}
