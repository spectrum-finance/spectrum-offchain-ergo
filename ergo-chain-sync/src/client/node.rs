use async_trait::async_trait;
use derive_more::From;
use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergo_chain_types::{BlockId, Header};
use isahc::{AsyncReadResponseExt, HttpClient};
use log::info;
use thiserror::Error;

use crate::client::model::{ApiInfo, FullBlock};
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
    async fn fetch_mempool(&self, offset: usize, limit: usize) -> Result<Vec<Transaction>, Error>;
    async fn get_best_height(&self) -> Result<u32, Error>;
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
        // info!("height: ${:} -> blocks ${:?}", height, blocks);
        if !blocks.is_empty() {
            let header_id = base16::encode_lower(&blocks[0].0 .0);
            // info!("height: ${:} -> header_id ${:?}", height, header_id);
            let transactions_path = format!("/blocks/{}/transactions", header_id);
            let mut resp = self
                .client
                .get_async(with_path(&self.base_url, &transactions_path))
                .await?;
            let s = resp.json::<BlockTransactions>().await;
            // info!("height: ${:} -> resp ${:?}", height, s);
            let block_transactions = if resp.status().is_success() {
                resp.json::<BlockTransactions>().await?
            } else {
                return Err(Error::UnsuccessfulRequest(
                    "expected 200 from /blocks/_/transactions".into(),
                ));
            };
            // info!("height: ${:} -> block_transactions ${:?}", height, block_transactions);

            let header_path = format!("/blocks/{}/header", header_id);
            let mut resp = self
                .client
                .get_async(with_path(&self.base_url, &header_path))
                .await?;
            let r = resp.json::<Header>().await;
            // info!("height: ${:} -> resp header ${:?}", height, r);
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

    async fn get_best_height(&self) -> Result<u32, Error> {
        let genesis_height = ApiInfo { fullHeight: 0 };
        let mut response = self
            .client
            .get_async(with_path(&self.base_url, &format!("/info")))
            .await?;
        let r = response.json::<ApiInfo>().await;
        info!("get_best_height -> ${:?}", r);
        let height = if response.status().is_success() {
            let r = response.json::<ApiInfo>().await;
            info!("get_best_height -> ${:?}", r);
            response
                .json::<ApiInfo>()
                .await
                .unwrap_or(genesis_height)
                .fullHeight
        } else {
            return Err(Error::UnsuccessfulRequest("expected 200 from /info".into()));
        };
        Ok(height)
    }

    async fn fetch_mempool(&self, offset: usize, limit: usize) -> Result<Vec<Transaction>, Error> {
        info!("Going to fetch mempool: offset {:}, limit {:}", offset, limit);
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
                "expected 200 from /transactions/unconfirmed?offset=_&limit=_".into(),
            ));
        };
        info!("New transactions from mempool {:}", transactions.len());
        Ok(transactions)
    }
}
