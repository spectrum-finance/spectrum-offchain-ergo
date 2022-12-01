use async_trait::async_trait;

use crate::backlog::data::BacklogOrder;
use crate::data::OnChainOrder;

#[async_trait(?Send)]
pub trait BacklogStore<TOrd>
where
    TOrd: OnChainOrder,
{
    async fn put(&mut self, ord: BacklogOrder<TOrd>);
    async fn exists(&self, ord: TOrd) -> bool;
    async fn drop(&mut self, ord_id: TOrd::TOrderId);
    async fn get(&self, ord_id: TOrd::TOrderId) -> Option<BacklogOrder<TOrd>>;
}
