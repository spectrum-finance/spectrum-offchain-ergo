use std::fmt::Display;
use std::hash::Hash;
use type_equalities::IsEqual;

pub mod state;
pub mod event;

pub trait Has<T> {
    fn get<U: IsEqual<T>>(&self) -> T;
}

pub trait Entity {
    type TEntityId: Eq + Hash + Display;
    type TStateId: Eq + Hash + Display;
}
