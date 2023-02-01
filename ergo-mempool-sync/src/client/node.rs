use ergo_chain_sync::client::model::FullBlock;
use ergo_lib::chain::transaction::Transaction;
use async_trait::async_trait;
use ergo_lib::ergo_chain_types::{BlockId, Header};
use isahc::{AsyncReadResponseExt, HttpClient};
use ergo_chain_sync::client::types::{Url, with_path};

use crate::client::models::ApiInfo;

#[async_trait::async_trait(? Send)]
pub trait ErgoNetwork {
    async fn fetch_mempool(&self, offset: usize, limit: usize) -> Vec<Transaction>;
    async fn get_best_height(&self) -> u32;
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
    async fn get_best_height(&self) -> u32 {
        self
            .client
            .get_async(with_path(&self.base_url, &format!("/info")))
            .await
            .ok().unwrap()
            .json::<ApiInfo>()
            .await
            .ok().unwrap()
            .fullHeight
    }

    async fn fetch_mempool(&self, offset: usize, limit: usize) -> Vec<Transaction> {
        self
            .client
            .get_async(with_path(
                &self.base_url,
                &format!("/transactions/unconfirmed?offset={:?}&limit={:?}", offset, limit)
            ))
            .await
            .ok().unwrap()
            .json::<Vec<Transaction>>()
            .await
            .ok().unwrap()
    }
}
