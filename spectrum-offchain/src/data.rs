use std::hash::Hash;

use type_equalities::IsEqual;

pub mod order;
pub mod unique_entity;

pub trait Has<T> {
    fn get<U: IsEqual<T>>(&self) -> T;
}

pub trait OnChainOrder {
    type TOrderId: Eq + Hash;
}
