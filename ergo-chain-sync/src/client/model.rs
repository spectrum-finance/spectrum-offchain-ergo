use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergo_chain_types::Header;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlockTransactions {
    pub transactions: Vec<Transaction>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FullBlock {
    pub header: Header,
    #[serde(rename = "blockTransactions")]
    pub block_transactions: BlockTransactions,
}
