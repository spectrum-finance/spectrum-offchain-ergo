use async_trait::async_trait;
use derive_more::Display;
use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use ergo_lib::ergotree_ir::chain::token::TokenId;
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
    async fn get_token_minting_info(
        &self,
        token_id: TokenId,
    ) -> Result<Option<TokenMintingInfo>, ClientError>;
    async fn box_in_utxo(&self, box_id: BoxId) -> bool;
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TokenMintingInfo {
    pub name: String,
    pub description: String,
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
        let mut res = self
            .client
            .send_async(req)
            .await
            .map_err(|e| ClientError(format!("ErgoNetwork::submit_tx: {:?}", e)))?;
        if res.status().is_client_error() {
            let details = res
                .json::<NodeError>()
                .await
                .ok()
                .map(|ne| format!("[{}] [{}] [{}]", ne.error, ne.reason, ne.detail))
                .unwrap_or("<unknown>".to_string());
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

    async fn get_token_minting_info(
        &self,
        token_id: TokenId,
    ) -> Result<Option<TokenMintingInfo>, ClientError> {
        let resp = self
            .client
            .get_async(with_path(
                &self.base_url,
                &format!("/blockchain/token/byId/{}", String::from(token_id)),
            ))
            .await
            .ok();

        if let Some(mut resp) = resp {
            let status_code = resp.status();
            if status_code.is_success() {
                Ok(resp.json::<TokenMintingInfo>().await.ok())
            } else {
                Err(ClientError(format!(
                    "expected 200 from /blockchain/token/byId/_, got {:?}",
                    status_code
                )))
            }
        } else {
            Err(ClientError("No response from ergo node".into()))
        }
    }

    async fn box_in_utxo(&self, box_id: BoxId) -> bool {
        let box_id_str = String::from(box_id);
        let resp = self
            .client
            .get_async(with_path(&self.base_url, &format!("/utxo/byId/{}", box_id_str)))
            .await
            .ok();
        if let Some(mut resp) = resp {
            let is_success = resp.status().is_success();
            let _ = resp.consume().await;
            return is_success;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use crate::network::ErgoNetwork;
    use ergo_chain_sync::client::{
        node::{ErgoNetwork as EN, ErgoNodeHttpClient},
        types::Url,
    };
    use ergo_lib::{ergo_chain_types::Digest32, ergotree_ir::chain::token::TokenId};
    use isahc::{prelude::Configurable, HttpClient};

    #[tokio::test]
    async fn test_token_minting_info() {
        let client = HttpClient::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap();
        let node = ErgoNodeHttpClient::new(
            client,
            Url::try_from(String::from("http://213.239.193.208:9053")).unwrap(),
        );
        let token_id = TokenId::try_from(
            Digest32::try_from(String::from(
                "9a06d9e545a41fd51eeffc5e20d818073bf820c635e2a9d922269913e0de369d",
            ))
            .unwrap(),
        )
        .unwrap();
        let info = node.get_token_minting_info(token_id).await.unwrap().unwrap();
        println!("{:?}", info);
    }

    #[tokio::test]
    async fn test_ergo_state_context() {
        let client = HttpClient::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap();
        let node = ErgoNodeHttpClient::new(
            client,
            Url::try_from(String::from("http://213.239.193.208:9053")).unwrap(),
        );
        let ctx = node.get_ergo_state_context().await.unwrap();
        println!("{:?}", ctx);
    }
}
