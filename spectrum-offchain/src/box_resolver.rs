use async_trait::async_trait;

use crate::box_resolver::repository::EntityRepo;
use crate::data::state::{Confirmed, Predicted, Traced, Unconfirmed};
use crate::data::Has;

pub mod process;
pub mod repository;

#[async_trait(?Send)]
pub trait BoxResolver<TEntity, TEntityId, TStateId> {
    /// Get resolved state of an on-chain entity `TEntity`.
    async fn get<'a>(&self, id: TEntityId) -> Option<TEntity>
    where
        TEntityId: 'a;
    /// Put predicted state of an on-chain entity `TEntity`.
    async fn put<'a>(&mut self, entity: Traced<Predicted<TEntity>, TStateId>)
    where
        TEntity: 'a,
        TStateId: 'a;
    /// Invalidate known state of an on-chain entity `TEntity`.
    async fn invalidate<'a>(&mut self, eid: TEntityId, sid: TStateId)
    where
        TEntityId: 'a,
        TStateId: 'a;
}

pub struct BoxResolverImpl<TRepo> {
    persistence: TRepo,
}

impl<TRepo> BoxResolverImpl<TRepo> {
    async fn is_linking<TEntity, TEntityId, TStateId>(&self, sid: TStateId, anchoring_sid: TStateId) -> bool
    where
        TRepo: EntityRepo<TEntity, TEntityId, TStateId>,
        TStateId: Eq,
    {
        let mut head_sid = sid;
        loop {
            match self.persistence.get_prediction(head_sid).await {
                None => return false,
                Some(Traced { prev_state_id, .. }) if prev_state_id == anchoring_sid => return true,
                Some(Traced { prev_state_id, .. }) => head_sid = prev_state_id,
            }
        }
    }
}

#[async_trait(?Send)]
impl<TEntity, TEntityId, TStateId, TRepo> BoxResolver<TEntity, TEntityId, TStateId> for BoxResolverImpl<TRepo>
where
    TRepo: EntityRepo<TEntity, TEntityId, TStateId>,
    TEntity: Has<TStateId>,
    TEntityId: Clone,
    TStateId: Eq,
{
    async fn get<'a>(&self, id: TEntityId) -> Option<TEntity>
    where
        TEntityId: 'a,
    {
        let confirmed = self.persistence.get_last_confirmed(id.clone()).await;
        let unconfirmed = self.persistence.get_last_unconfirmed(id.clone()).await;
        let predicted = self.persistence.get_last_predicted(id).await;
        match (confirmed, unconfirmed, predicted) {
            (Some(Confirmed(conf)), unconf, Some(Predicted(pred))) => {
                let anchoring_point = unconf.map(|Unconfirmed(e)| e).unwrap_or(conf);
                let anchoring_sid = anchoring_point.get::<TStateId>();
                let predicted_sid = pred.get::<TStateId>();
                let prediction_is_anchoring_point = predicted_sid == anchoring_sid;
                let prediction_is_valid =
                    prediction_is_anchoring_point || self.is_linking(predicted_sid, anchoring_sid).await;
                let safe_point = if prediction_is_valid {
                    pred
                } else {
                    anchoring_point
                };
                Some(safe_point)
            }
            (_, Some(Unconfirmed(unconf)), None) => Some(unconf),
            (Some(Confirmed(conf)), _, _) => Some(conf),
            _ => None,
        }
    }

    async fn put<'a>(&mut self, entity: Traced<Predicted<TEntity>, TStateId>)
    where
        TEntity: 'a,
        TStateId: 'a,
    {
        self.persistence.put_predicted(entity).await;
    }

    async fn invalidate<'a>(&mut self, eid: TEntityId, sid: TStateId)
    where
        TEntityId: 'a,
        TStateId: 'a,
    {
        self.persistence.invalidate(eid, sid).await;
    }
}
