use async_trait::async_trait;
use derive_more::From;
use ergo_lib::ergo_chain_types::{BlockId, Header};
use isahc::{AsyncReadResponseExt, HttpClient};
use thiserror::Error;

use crate::client::model::FullBlock;
use crate::client::types::Url;

use super::model::BlockTransactions;
use super::types::with_path;

#[derive(Error, From, Debug)]
pub enum Error {
    #[error("json decoding: {0}")]
    Json(serde_json::Error),
    #[error("isahc: {0}")]
    Isahc(isahc::Error),
    #[error("unsuccessful request: {0}")]
    UnsuccessfulRequest(String),
    #[error("No block found")]
    NoBlock,
}

#[async_trait(?Send)]
pub trait ErgoNetwork {
    async fn get_block_at(&self, height: u32) -> Result<FullBlock, Error>;
}

#[derive(Clone)]
pub struct ErgoNodeHttpClient {
    pub client: HttpClient,
    pub base_url: Url,
}

impl ErgoNodeHttpClient {
    pub fn new(client: HttpClient, base_url: Url) -> Self {
        Self { client, base_url }
    }
}

#[async_trait(?Send)]
impl ErgoNetwork for ErgoNodeHttpClient {
    async fn get_block_at(&self, height: u32) -> Result<FullBlock, Error> {
        let blocks = self
            .client
            .get_async(with_path(&self.base_url, &format!("/blocks/at/{}", height)))
            .await?
            .json::<Vec<BlockId>>()
            .await?;
        if !blocks.is_empty() {
            let header_id = base16::encode_lower(&blocks[0].0 .0);
            let transactions_path = format!("/blocks/{}/transactions", header_id);
            let mut resp = self
                .client
                .get_async(with_path(&self.base_url, &transactions_path))
                .await?;
            let block_transactions = if resp.status().is_success() {
                resp.json::<BlockTransactions>().await?
            } else {
                return Err(Error::UnsuccessfulRequest(
                    "expected 200 from /blocks/_/transactions".into(),
                ));
            };

            let header_path = format!("/blocks/{}/header", header_id);
            let mut resp = self
                .client
                .get_async(with_path(&self.base_url, &header_path))
                .await?;
            let header = if resp.status().is_success() {
                resp.json::<Header>().await?
            } else {
                return Err(Error::UnsuccessfulRequest(
                    "expected 200-200 from /blocks/_/header".into(),
                ));
            };
            Ok(FullBlock {
                header,
                block_transactions,
            })
        } else {
            Err(Error::NoBlock)
        }
    }
}
