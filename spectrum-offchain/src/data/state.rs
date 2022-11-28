/// A unique, persistent, self-reproducible, on-chiain entity.
pub struct Traced<TEntity, TStateId> {
    pub state: TEntity,
    pub prev_state_id: TStateId,
}

pub struct Predicted<T>(pub T);

pub struct Confirmed<T>(pub T);

pub struct Unconfirmed<T>(pub T);