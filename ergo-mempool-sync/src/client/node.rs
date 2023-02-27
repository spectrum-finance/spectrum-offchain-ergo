use ergo_lib::chain::transaction::Transaction;
use async_trait::async_trait;
use isahc::{AsyncReadResponseExt, HttpClient};
use ergo_chain_sync::client::types::{Url, with_path};
use derive_more::From;
use thiserror::Error;

use crate::client::models::ApiInfo;

#[derive(Error, From, Debug)]
pub enum Error {
    #[error("json decoding: {0}")]
    Json(serde_json::Error),
    #[error("isahc: {0}")]
    Isahc(isahc::Error),
    #[error("unsuccessful request: {0}")]
    UnsuccessfulRequest(String)
}

#[async_trait::async_trait(? Send)]
pub trait ErgoNetwork {
    async fn fetch_mempool(&self, offset: usize, limit: usize) -> Result<Vec<Transaction>, Error>;
    async fn get_best_height(&self) -> Result<u32, Error>;
}

#[derive(Clone)]
pub struct ErgoMempoolHttpClient {
    pub client: HttpClient,
    pub base_url: Url,
}

impl ErgoMempoolHttpClient {
    pub fn new(client: HttpClient, base_url: Url) -> Self {
        Self { client, base_url }
    }
}

#[async_trait(? Send)]
impl ErgoNetwork for ErgoMempoolHttpClient {
    async fn get_best_height(&self) -> Result<u32, Error> {
        let genesis_height = ApiInfo { full_height: 0 };
        let mut response = self
            .client
            .get_async(with_path(&self.base_url, &format!("/info")))
            .await?;
        let height = if response.status().is_success() {
            response.json::<ApiInfo>().await.unwrap_or(genesis_height).full_height
        } else {
            return Err(Error::UnsuccessfulRequest(
                "expected 200 from /info".into())
            );
        };
        Ok(height)
    }

    async fn fetch_mempool(&self, offset: usize, limit: usize) -> Result<Vec<Transaction>, Error> {
        let mut response = self
            .client
            .get_async(with_path(
                &self.base_url,
                &format!("/transactions/unconfirmed?offset={:?}&limit={:?}", offset, limit),
            ))
            .await?;
        let transactions = if response.status().is_success() {
            response.json::<Vec<Transaction>>().await?
        } else {
            return Err(Error::UnsuccessfulRequest(
                "expected 200 from /transactions/unconfirmed?offset=_&limit=_".into())
            );
        };
        Ok(transactions)
    }
}
