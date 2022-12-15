use std::sync::Arc;

use parking_lot::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::data::unique_entity::{Confirmed, Predicted, Traced, Unconfirmed};
use crate::data::OnChainEntity;

pub mod persistence;
pub mod process;
pub mod rocksdb;

/// Get latest state of an on-chain entity `TEntity`.
pub async fn resolve_entity_state<TEntity, TRepo>(
    id: TEntity::TEntityId,
    repo: Arc<Mutex<TRepo>>,
) -> Option<TEntity>
where
    TRepo: EntityRepo<TEntity>,
    TEntity: OnChainEntity,
    TEntity::TEntityId: Clone,
{
    let repo_guard = repo.lock();
    let confirmed = repo_guard.get_last_confirmed(id.clone()).await;
    let unconfirmed = repo_guard.get_last_unconfirmed(id.clone()).await;
    let predicted = repo_guard.get_last_predicted(id).await;
    match (confirmed, unconfirmed, predicted) {
        (Some(Confirmed(conf)), unconf, Some(Predicted(pred))) => {
            let anchoring_point = unconf.map(|Unconfirmed(e)| e).unwrap_or(conf);
            let anchoring_sid = anchoring_point.get_self_state_ref();
            let predicted_sid = pred.get_self_state_ref();
            let prediction_is_anchoring_point = predicted_sid == anchoring_sid;
            let prediction_is_valid = prediction_is_anchoring_point
                || is_linking(predicted_sid, anchoring_sid, Arc::clone(&repo)).await;
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

async fn is_linking<TEntity, TRepo>(
    sid: TEntity::TStateId,
    anchoring_sid: TEntity::TStateId,
    persistence: Arc<Mutex<TRepo>>,
) -> bool
where
    TEntity: OnChainEntity,
    TRepo: EntityRepo<TEntity>,
{
    let mut head_sid = sid;
    loop {
        match persistence.lock().get_prediction_predecessor(head_sid).await {
            None => return false,
            Some(prev_state_id)
                if prev_state_id
                    .as_ref()
                    .map(|prev_sid| *prev_sid == anchoring_sid)
                    .unwrap_or(false) =>
            {
                return true
            }
            Some(prev_state_id) => {
                if let Some(prev_sid) = prev_state_id {
                    head_sid = prev_sid
                } else {
                    return false;
                }
            }
        }
    }
}
