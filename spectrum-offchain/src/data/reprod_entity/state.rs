/// A unique, persistent, self-reproducible, on-chiain entity.
#[derive(Debug, Clone)]
pub struct Traced<TEntity, TStateId> {
    pub state: TEntity,
    pub prev_state_id: TStateId,
}

#[derive(Debug, Clone)]
pub struct Predicted<T>(pub T);

#[derive(Debug, Clone)]
pub struct Confirmed<T>(pub T);

#[derive(Debug, Clone)]
pub struct Unconfirmed<T>(pub T);