use ergo_chain_sync::client::model::FullBlock;
use ergo_lib::chain::transaction::Transaction;

#[async_trait::async_trait(?Send)]
pub trait ErgoNetwork {
    async fn fetch_mempool(&self, offset: usize, limit: usize) -> Vec<Transaction>;
    async fn fetch_last_blocks(&self, n: usize) -> Vec<FullBlock>;
    async fn get_best_height(&self) -> u32;
}
