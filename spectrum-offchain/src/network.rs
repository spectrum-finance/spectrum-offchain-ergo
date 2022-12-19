use async_trait::async_trait;
use derive_more::Display;
use ergo_lib::chain::transaction::Transaction;

#[derive(Debug, Display)]
pub struct ClientError(String);

#[async_trait]
pub trait ErgoNetwork {
    /// Submit the given `Transaction` to Ergo network.
    async fn submit_tx(&self, tx: Transaction) -> Result<(), ClientError>;
}
