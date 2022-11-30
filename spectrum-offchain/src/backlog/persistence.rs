use async_trait::async_trait;

use crate::backlog::data::BacklogOrder;

#[async_trait(?Send)]
pub trait BacklogStore<TOrd, TOrdId> {
    async fn put(&mut self, ord: BacklogOrder<TOrd>);
    async fn exists(&self, ord_id: TOrdId) -> bool;
    async fn drop(&mut self, ord_id: TOrdId);
    async fn get(&self, ord_id: TOrdId) -> Option<BacklogOrder<TOrd>>;
    async fn get_all(&self) -> Vec<BacklogOrder<TOrd>>;
}
