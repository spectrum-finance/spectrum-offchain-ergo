use async_trait::async_trait;
use ergo_lib::ergo_chain_types::BlockId;
use reqwest::Client;

use crate::client::model::FullBlock;
use crate::client::types::Url;

#[async_trait]
pub trait ErgoNetwork {
    async fn get_block_at(&self, height: u32) -> Option<FullBlock>;
}

pub struct ErgoNodeHttpClient {
    client: Client,
    base_url: Url,
}

impl ErgoNodeHttpClient {
    pub fn new(client: Client, base_url: Url) -> Self {
        Self { client, base_url }
    }
}

#[async_trait]
impl ErgoNetwork for ErgoNodeHttpClient {
    async fn get_block_at(&self, height: u32) -> Option<FullBlock> {
        let blocks = self
            .client
            .get(format!("{}/blocks/at/{}", self.base_url, height))
            .send()
            .await
            .ok()?
            .json::<Vec<BlockId>>()
            .await
            .ok()?;
        if !blocks.is_empty() {
            let resp = self
                .client
                .get(format!(
                    "{}/blocks/{}",
                    self.base_url,
                    base16::encode_lower(&*blocks[0].0 .0)
                ))
                .send()
                .await
                .ok()?;
            if resp.status().is_success() {
                return resp.json::<FullBlock>().await.ok();
            }
        }
        None
    }
}
