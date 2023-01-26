use async_std::prelude::FutureExt;
use async_trait::async_trait;
use ergo_lib::ergo_chain_types::{BlockId, Header};
use isahc::{AsyncReadResponseExt, HttpClient};

use crate::client::model::FullBlock;
use crate::client::types::Url;

use super::model::BlockTransactions;
use super::types::with_path;

#[async_trait(?Send)]
pub trait ErgoNetwork {
    async fn get_block_at(&self, height: u32) -> Option<FullBlock>;
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
    async fn get_block_at(&self, height: u32) -> Option<FullBlock> {
        let blocks = self
            .client
            .get_async(with_path(&self.base_url, &format!("/blocks/at/{}", height)))
            .await
            .ok()?
            .json::<Vec<BlockId>>()
            .await
            .ok()?;
        if !blocks.is_empty() {
            let header_id = base16::encode_lower(&blocks[0].0 .0);
            let transactions_path = format!("/blocks/{}/transactions", header_id);
            let mut resp = self
                .client
                .get_async(with_path(&self.base_url, &transactions_path))
                .await
                .ok()?;
            println!("Response txn is: {:?}", resp.status().clone());
            let block_transactions = if resp.status().is_success() {
                let a = resp.json::<BlockTransactions>().await;
                println!("Parse txns: ${:?}", a.as_ref());
                a.ok()?
            } else {
                return None;
            };

            let header_path = format!("/blocks/{}/header", header_id);
            let mut resp = self
                .client
                .get_async(with_path(&self.base_url, &header_path))
                .await
                .ok()?;
            println!("Response header is: {:?}", resp.status().clone());
            let header = if resp.status().is_success() {
                resp.json::<Header>().await.ok()?
            } else {
                return None;
            };
            Some(FullBlock {
                header,
                block_transactions,
            })
        } else {
            None
        }
    }
}
