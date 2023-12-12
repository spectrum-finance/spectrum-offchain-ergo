use derive_more::From;
use ergo_chain_sync::client::types::{with_path, Url};
use ergo_lib::ergotree_ir::chain::{
    address::{Address, AddressEncoder, NetworkPrefix},
    ergo_box::{BoxId, ErgoBox},
};
use isahc::{AsyncReadResponseExt, HttpClient};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub struct Explorer {
    pub client: HttpClient,
    pub base_url: Url,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Items<A> {
    items: Vec<A>,
}

impl Explorer {
    pub async fn get_utxos(&self, addr: &Address) -> Result<Vec<ErgoBox>, ExplorerError> {
        Ok(self
            .client
            .get_async(with_path(
                &self.base_url,
                &format!(
                    "/api/v1/boxes/unspent/byAddress/{}",
                    AddressEncoder::encode_address_as_string(NetworkPrefix::Mainnet, addr),
                ),
            ))
            .await?
            .json::<Items<ErgoBox>>()
            .await?
            .items)
    }

    pub async fn get_box(&self, box_id: BoxId) -> Result<ErgoBox, ExplorerError> {
        let mut response = self
            .client
            .get_async(with_path(&self.base_url, &format!("/api/v1/boxes/{}", box_id)))
            .await?;
        let ergo_box = if response.status().is_success() {
            response.json::<ErgoBox>().await?
        } else {
            return Err(ExplorerError::UnsuccessfulRequest(
                "expected 200 from /boxes/".into(),
            ));
        };
        Ok(ergo_box)
    }
}

#[derive(Error, From, Debug)]
pub enum ExplorerError {
    #[error("json decoding: {0}")]
    Json(serde_json::Error),
    #[error("isahc: {0}")]
    Isahc(isahc::Error),
    #[error("unsuccessful request: {0}")]
    UnsuccessfulRequest(String),
    #[error("No Box found")]
    NoBox,
}
