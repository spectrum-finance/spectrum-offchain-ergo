use async_trait::async_trait;
use derive_more::Display;
use ergo_lib::chain::transaction::Transaction;
use isahc::AsyncReadResponseExt;
use isahc::Request;
use serde::{Deserialize, Serialize};

use ergo_chain_sync::client::node::ErgoNodeHttpClient;
use ergo_chain_sync::client::types::with_path;

#[derive(Debug, Display)]
pub struct ClientError(pub String);

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeError {
    error: u16,
    reason: String,
    detail: String,
}

#[async_trait]
pub trait ErgoNetwork {
    /// Submit the given `Transaction` to Ergo network.
    async fn submit_tx(&self, tx: Transaction) -> Result<(), ClientError>;
    async fn get_height(&self) -> u32;
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NodeStatus {
    #[serde(rename = "fullHeight")]
    full_height: u32,
}

const GENESIS_HEIGHT: u32 = 0;

#[async_trait]
impl ErgoNetwork for ErgoNodeHttpClient {
    async fn submit_tx(&self, tx: Transaction) -> Result<(), ClientError> {
        println!("{}", serde_json::to_string(&tx).unwrap());
        let req = Request::post(with_path(&self.base_url, "/transactions"))
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&tx).unwrap())
            .unwrap();
        let mut res = self.client.send_async(req).await.unwrap();
        if res.status().is_client_error() {
            let details = res
                .json::<NodeError>()
                .await
                .ok()
                .map(|ne| format!("[{}] [{}] [{}]", ne.error, ne.reason, ne.detail))
                .unwrap_or(format!("<unknown>"));
            Err(ClientError(format!("Malformed tx. {}", details)))
        } else {
            Ok(())
        }
    }

    async fn get_height(&self) -> u32 {
        let resp = self
            .client
            .get_async(with_path(&self.base_url, "/info"))
            .await
            .ok();
        if let Some(mut resp) = resp {
            if resp.status().is_success() {
                return resp
                    .json::<NodeStatus>()
                    .await
                    .ok()
                    .map(|b| b.full_height)
                    .unwrap_or(GENESIS_HEIGHT);
            }
        }
        GENESIS_HEIGHT
    }
}
