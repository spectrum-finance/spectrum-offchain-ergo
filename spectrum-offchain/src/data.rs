use type_equalities::IsEqual;

pub mod state;

pub trait Has<T> {
    fn get<U: IsEqual<T>>(&self) -> T;
}
