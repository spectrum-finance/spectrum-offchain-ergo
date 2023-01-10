use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergo_chain_types::Header;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlockTransactions {
    pub transactions: Vec<Transaction>,
}

#[derive(Debug, Clone)]
pub struct FullBlock {
    pub header: Header,
    pub block_transactions: BlockTransactions,
}
