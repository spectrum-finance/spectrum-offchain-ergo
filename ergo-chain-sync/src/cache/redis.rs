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

    /// Clears out the entire redis DB
    pub async fn flush_db(&self) {
        let mut conn = self.pool.get().await.unwrap();
        let _: () = cmd("FLUSHDB").query_async(&mut conn).await.unwrap();
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
            .arg(&parent_id_hex)
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

        // DEL best_block
        let _: () = cmd("DEL").arg(BEST_BLOCK).query_async(&mut conn).await.unwrap();

        // RPUSH best_block {block_id_hex} {parent_id_hex}
        let _: () = cmd("RPUSH")
            .arg(BEST_BLOCK)
            .arg(&block_id_hex)
            .arg(&parent_id_hex)
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
        let block_id_hex: Vec<String> = cmd("LRANGE")
            .arg(BEST_BLOCK)
            .arg(0)
            .arg(0)
            .query_async(&mut conn)
            .await
            .unwrap();
        if !block_id_hex.is_empty() {
            let digest32 = Digest32::try_from(block_id_hex[0].clone()).unwrap();
            let id = BlockId(digest32);
            let height: u32 = cmd("GET")
                .arg(&concat_string!(block_id_hex[0], ":h"))
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
            let block_id_digest32 = Digest32::try_from(block_id_hex.clone()).unwrap();
            let block_id = BlockId(block_id_digest32);

            let parent_id_hex: String = cmd("GET")
                .arg(concat_string!(block_id_hex, ":p"))
                .query_async(&mut conn)
                .await
                .unwrap();
            let parent_digest32 = Digest32::try_from(parent_id_hex.clone()).unwrap();
            let parent_id = BlockId(parent_digest32);

            // The new best block will now be the parent of the old best block.
            if let Ok(new_parent_id_hex) = cmd("GET")
                .arg(concat_string!(parent_id_hex, ":p"))
                .query_async::<_, String>(&mut conn)
                .await
            {
                // RPUSH best_block {parent_id_hex} {new_parent_id_hex}
                let _: () = cmd("RPUSH")
                    .arg(BEST_BLOCK)
                    .arg(&parent_id_hex)
                    .arg(&new_parent_id_hex)
                    .query_async(&mut conn)
                    .await
                    .unwrap();
            }

            let transaction_ids: Vec<String> = cmd("SMEMBERS")
                .arg(concat_string!(block_id_hex, ":t"))
                .query_async(&mut conn)
                .await
                .unwrap();

            let mut transactions = Vec::with_capacity(transaction_ids.len());
            for tx_id_hex in transaction_ids {
                let tx_json: String = cmd("GETDEL")
                    .arg(&tx_id_hex)
                    .query_async(&mut conn)
                    .await
                    .unwrap();
                let tx: Transaction = serde_json::from_str(&tx_json).unwrap();
                transactions.push(tx);
            }

            // DEL best_block
            let _: () = cmd("DEL").arg(BEST_BLOCK).query_async(&mut conn).await.unwrap();

            Some(Block {
                id: block_id,
                parent_id,
                height,
                transactions,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use ergo_lib::chain::transaction::Transaction;
    use ergo_lib::ergo_chain_types::{BlockId, Digest32};
    use sigma_test_util::force_any_val;

    use crate::cache::chain_cache::InMemoryCache;
    use crate::model::Block;

    #[async_std::test]
    async fn test_redis() {
        let mut client = RedisClient::new("redis://127.0.0.1/");
        test_client(client).await;
    }

    async fn test_client<C: ChainCache>(mut client: C) {
        let block_ids: Vec<_> = force_any_val::<[Digest32; 30]>()
            .into_iter()
            .map(BlockId)
            .collect();
        let mut height = 1;

        let first_id = block_ids[0].clone();
        let mut blocks = vec![];

        for i in 1..30 {
            let transactions = force_any_val::<[Transaction; 10]>().to_vec();
            let parent_id = block_ids[i - 1].clone();
            let id = block_ids[i].clone();
            let block = Block {
                id: id.clone(),
                parent_id,
                height,
                transactions,
            };
            blocks.push(block.clone());
            height += 1;

            client.append_block(block).await;
            assert!(client.exists(id).await);
        }

        assert!(!client.exists(first_id).await);

        // Now pop off best blocks
        while let Some(b0) = client.take_best_block().await {
            let b1 = blocks.pop().unwrap();
            assert_eq!(b0.id, b1.id);
            assert_eq!(b0.parent_id, b1.parent_id);
            assert_eq!(b0.height, b1.height);

            // Check that the collection of transactions coincide.
            assert_eq!(b0.transactions.len(), b1.transactions.len());
            for tx0 in b0.transactions {
                let tx1 = b1.transactions.iter().find(|t| tx0.id() == t.id()).unwrap();
                assert_eq!(tx0, *tx1);
            }
        }
    }

    #[async_std::test]
    async fn test_inmemory_cache() {
        test_client(InMemoryCache::new()).await;
    }
}
