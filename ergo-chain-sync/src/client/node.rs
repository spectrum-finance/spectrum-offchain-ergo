use async_trait::async_trait;
use ergo_lib::chain::ergo_state_context::ErgoStateContext;
use ergo_lib::ergo_chain_types::{BlockId, Header, PreHeader};
use isahc::{AsyncReadResponseExt, HttpClient};

use crate::client::model::FullBlock;
use crate::client::types::Url;

#[async_trait(?Send)]
pub trait ErgoNetwork {
    async fn get_block_at(&self, height: u32) -> Option<FullBlock>;
    async fn get_ergo_state_context(&self) -> Option<ErgoStateContext>;
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
            .get_async(format!("{}/blocks/at/{}", self.base_url, height))
            .await
            .ok()?
            .json::<Vec<BlockId>>()
            .await
            .ok()?;
        if !blocks.is_empty() {
            let mut resp = self
                .client
                .get_async(format!(
                    "{}/blocks/{}",
                    self.base_url,
                    base16::encode_lower(&blocks[0].0 .0)
                ))
                .await
                .ok()?;
            if resp.status().is_success() {
                return resp.json::<FullBlock>().await.ok();
            }
        }
        None
    }

    async fn get_ergo_state_context(&self) -> Option<ErgoStateContext> {
        let headers = self
            .client
            .get_async(format!("{}/blocks/lastHeaders/{}", self.base_url, 10)) // `ErgoStateContext` needs the 10 last Headers
            .await
            .ok()?
            .json::<Vec<Header>>()
            .await
            .ok()?;

        // Note that the `Header`s are returned in ascending-height order, and we derive the
        // `PreHeader` from latest `Header`.
        let pre_header = PreHeader::from(headers.last().unwrap().clone());
        Some(ErgoStateContext {
            pre_header,
            headers: headers.try_into().unwrap(),
        })
    }
}
