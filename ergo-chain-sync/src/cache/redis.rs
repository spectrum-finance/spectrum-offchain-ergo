use concat_string::concat_string;
use deadpool_redis::{Config, Pool, Runtime};
use ergo_lib::{
    chain::transaction::Transaction,
    ergo_chain_types::{BlockId, Digest32},
};
use redis::cmd;

use crate::model::{Block, BlockRecord};
use async_trait::async_trait;

use super::chain_cache::ChainCache;

static BEST_BLOCK: &str = "best_block";

struct RedisClient {
    pool: Pool,
}

impl RedisClient {
    pub fn new(url: &str) -> Self {
        let cfg = Config::from_url(url);
        let pool = cfg.create_pool(Some(Runtime::AsyncStd1)).unwrap();
        Self { pool }
    }
}

#[async_trait(?Send)]
impl ChainCache for RedisClient {
    async fn append_block(&mut self, block: Block) {
        let mut conn = self.pool.get().await.unwrap();
        let block_id_hex = String::from(block.id.0.clone());
        let parent_id_hex = String::from(block.parent_id.0.clone());
        let parent_id_key = concat_string!(block_id_hex, ":p");
        let block_height_key = concat_string!(block_id_hex, ":h");
        let block_transactions_key = concat_string!(block_id_hex, ":t");

        // SET {block_id_hex}:p {parent_id_hex}
        let _: () = cmd("SET")
            .arg(&parent_id_key)
            .arg(parent_id_hex)
            .query_async(&mut conn)
            .await
            .unwrap();

        // SET {block_id_hex}:h {block.height}
        let _: () = cmd("SET")
            .arg(&block_height_key)
            .arg(block.height)
            .query_async(&mut conn)
            .await
            .unwrap();

        for tx in block.transactions {
            let tx_json = serde_json::to_string(&tx).unwrap();
            let tx_id = String::from(tx.id().0);

            // SADD {block_id_hex}:t {tx_id}
            let _: () = cmd("SADD")
                .arg(&block_transactions_key)
                .arg(&tx_id)
                .query_async(&mut conn)
                .await
                .unwrap();

            // SET {tx_id} {tx_json}
            let _: () = cmd("SET")
                .arg(&tx_id)
                .arg(&tx_json)
                .query_async(&mut conn)
                .await
                .unwrap();
        }

        // SET best_block {block_id_hex}
        let _: () = cmd("SET")
            .arg(BEST_BLOCK)
            .arg(&block_id_hex)
            .query_async(&mut conn)
            .await
            .unwrap();
    }

    async fn exists(&mut self, block_id: BlockId) -> bool {
        let mut conn = self.pool.get().await.unwrap();
        let block_id_hex = String::from(block_id.0);
        let block_height_key = concat_string!(block_id_hex, ":h");
        let exists: bool = cmd("EXISTS")
            .arg(&block_height_key)
            .query_async(&mut conn)
            .await
            .unwrap();
        exists
    }

    async fn get_best_block(&mut self) -> Option<BlockRecord> {
        let mut conn = self.pool.get().await.unwrap();
        let block_id_hex: Option<String> = cmd("GET").arg(BEST_BLOCK).query_async(&mut conn).await.unwrap();
        if let Some(block_id_hex) = block_id_hex {
            let digest32 = Digest32::try_from(block_id_hex.clone()).unwrap();
            let id = BlockId(digest32);
            let height: u32 = cmd("GET")
                .arg(&concat_string!(block_id_hex, ":h"))
                .query_async(&mut conn)
                .await
                .unwrap();
            Some(BlockRecord { id, height })
        } else {
            None
        }
    }

    async fn take_best_block(&mut self) -> Option<Block> {
        let mut conn = self.pool.get().await.unwrap();
        if let Some(BlockRecord { id, height }) = self.get_best_block().await {
            let _: () = cmd("DEL").arg(BEST_BLOCK).query_async(&mut conn).await.unwrap();
            let block_id_hex = String::from(id.0.clone());

            let parent_id_hex: String = cmd("GET")
                .arg(concat_string!(block_id_hex, ":p"))
                .query_async(&mut conn)
                .await
                .unwrap();
            let parent_digest32 = Digest32::try_from(parent_id_hex).unwrap();
            let parent_id = BlockId(parent_digest32);

            let transaction_ids: Vec<String> = cmd("SMEMBERS")
                .arg(concat_string!(block_id_hex, ":t"))
                .query_async(&mut conn)
                .await
                .unwrap();

            let mut transactions = Vec::with_capacity(transaction_ids.len());
            for tx_id_hex in transaction_ids {
                let tx_json: String = cmd("GET").arg(&tx_id_hex).query_async(&mut conn).await.unwrap();
                let tx: Transaction = serde_json::from_str(&tx_json).unwrap();
                transactions.push(tx);
            }

            Some(Block {
                id,
                parent_id,
                height,
                transactions,
            })
        } else {
            None
        }
    }
}
