use async_trait::async_trait;

use crate::box_resolver::persistence::EntityRepo;
use crate::data::unique_entity::{Confirmed, Predicted, Traced, Unconfirmed};
use crate::data::OnChainEntity;

pub mod persistence;
pub mod process;

#[async_trait(?Send)]
pub trait BoxResolver<TEntity: OnChainEntity> {
    /// Get resolved state of an on-chain entity `TEntity`.
    async fn get<'a>(&self, id: TEntity::TEntityId) -> Option<TEntity>
    where
        TEntity::TEntityId: 'a;
    /// Put predicted state of an on-chain entity `TEntity`.
    async fn put<'a>(&mut self, entity: Traced<Predicted<TEntity>>)
    where
        TEntity: 'a;
    /// Invalidate known state of an on-chain entity `TEntity`.
    async fn invalidate<'a>(&mut self, eid: TEntity::TEntityId, sid: TEntity::TStateId)
    where
        TEntity::TEntityId: 'a,
        TEntity::TStateId: 'a;
}

pub struct BoxResolverImpl<TRepo> {
    persistence: TRepo,
}

impl<TRepo> BoxResolverImpl<TRepo> {
    async fn is_linking<TEntity>(&self, sid: TEntity::TStateId, anchoring_sid: TEntity::TStateId) -> bool
    where
        TEntity: OnChainEntity,
        TRepo: EntityRepo<TEntity>,
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
impl<TEntity, TRepo> BoxResolver<TEntity> for BoxResolverImpl<TRepo>
where
    TRepo: EntityRepo<TEntity>,
    TEntity: OnChainEntity,
    TEntity::TEntityId: Clone,
{
    async fn get<'a>(&self, id: TEntity::TEntityId) -> Option<TEntity>
    where
        TEntity::TEntityId: 'a,
    {
        let confirmed = self.persistence.get_last_confirmed(id.clone()).await;
        let unconfirmed = self.persistence.get_last_unconfirmed(id.clone()).await;
        let predicted = self.persistence.get_last_predicted(id).await;
        match (confirmed, unconfirmed, predicted) {
            (Some(Confirmed(conf)), unconf, Some(Predicted(pred))) => {
                let anchoring_point = unconf.map(|Unconfirmed(e)| e).unwrap_or(conf);
                let anchoring_sid = anchoring_point.get_self_state_ref();
                let predicted_sid = pred.get_self_state_ref();
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

    async fn put<'a>(&mut self, entity: Traced<Predicted<TEntity>>)
    where
        TEntity: 'a,
    {
        self.persistence.put_predicted(entity).await;
    }

    async fn invalidate<'a>(&mut self, eid: TEntity::TEntityId, sid: TEntity::TStateId)
    where
        TEntity::TEntityId: 'a,
        TEntity::TStateId: 'a,
    {
        self.persistence.invalidate(eid, sid).await;
    }
}
