use type_equalities::IsEqual;

pub mod order;
pub mod reprod_entity;

pub trait Has<T> {
    fn get<U: IsEqual<T>>(&self) -> T;
}
