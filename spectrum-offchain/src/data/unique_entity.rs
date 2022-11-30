/// A unique, persistent, self-reproducible, on-chiain entity.
#[derive(Debug, Clone)]
pub struct Traced<TEntity, TStateId> {
    pub state: TEntity,
    pub prev_state_id: TStateId,
}

/// Entity contexts:

/// State `T` is predicted, but not confirmed to be included into blockchain or mempool yet.
#[derive(Debug, Clone)]
pub struct Predicted<T>(pub T);

/// State `T` is confirmed to be included into blockchain.
#[derive(Debug, Clone)]
pub struct Confirmed<T>(pub T);

/// State `T` was observed in mempool.
#[derive(Debug, Clone)]
pub struct Unconfirmed<T>(pub T);

/// How states apply to the sequence of states of an entity:

/// State is applied on top of previous states.
#[derive(Debug, Clone)]
pub struct Upgrade<T>(pub T);

/// State is discarded and should be eliminated from the sequence of upgrades.
#[derive(Debug, Clone)]
pub struct UpgradeRollback<T>(pub T);
