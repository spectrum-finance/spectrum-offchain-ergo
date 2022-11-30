use async_trait::async_trait;
use crate::data::order::{PendingOrder, ProgressingOrder, SuspendedOrder};

pub(crate) mod persistence;
pub(crate) mod data;

#[async_trait(?Send)]
pub trait Backlog<TOrd, TOrdId> {
    async fn put(&mut self, ord: PendingOrder<TOrd>);
    async fn suspend(&mut self, ord: SuspendedOrder<TOrd>) -> bool;
    async fn check_later(&mut self, ord: ProgressingOrder<TOrd>) -> bool;
    async fn try_acquire(&mut self) -> Option<TOrd>;
    async fn drop(&mut self, ord_id: TOrdId);
}
