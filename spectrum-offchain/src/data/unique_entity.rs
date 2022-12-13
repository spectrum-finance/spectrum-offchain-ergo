use serde::{Deserialize, Serialize};

use crate::data::OnChainEntity;

/// A unique, persistent, self-reproducible, on-chiain entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Traced<TEntity: OnChainEntity> {
    pub state: TEntity,
    pub prev_state_id: Option<TEntity::TStateId>,
}

/// Entity contexts:

/// State `T` is predicted, but not confirmed to be included into blockchain or mempool yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Predicted<T>(pub T);

impl<T: OnChainEntity> OnChainEntity for Predicted<T> {
    type TEntityId = T::TEntityId;
    type TStateId = T::TStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.0.get_self_ref()
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.0.get_self_state_ref()
    }
}

/// State `T` is confirmed to be included into blockchain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Confirmed<T>(pub T);

/// State `T` was observed in mempool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unconfirmed<T>(pub T);

/// How states apply to the sequence of states of an entity:

/// State is applied on top of previous states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upgrade<T>(pub T);

/// State is discarded and should be eliminated from the sequence of upgrades.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeRollback<T>(pub T);
