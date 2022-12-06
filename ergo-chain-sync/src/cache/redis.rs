use async_trait::async_trait;
use concat_string::concat_string;
use deadpool_redis::{Config, Pool, Runtime};
use ergo_lib::{
    chain::transaction::Transaction,
    ergo_chain_types::{BlockId, Digest32},
};
use redis::cmd;

use crate::model::{Block, BlockRecord};

use super::chain_cache::ChainCache;

static BEST_BLOCK: &str = "best_block";

/// How the blocks are stored in Redis:
///
/// Given a block `B`, let `HB` denote the (lowercase) hex-representation of block's ID. Then
///  - {HB}:p is the key which maps to the hex-representation of B's parent block ID.
///  - {HB}:h is the key which maps to the height of `B`.
///  - {HB}:t is the key of a Redis set, which contains the hex-representation `HT` of the
///    transaction ID of every transaction of `B`.
///    - Every {HT} is a Redis key which maps to the string-encoded JSON representation of the
///      transaction.
///  - {BEST_BLOCK} is a key which maps to a Redis list of length 2, containing the
///    hex-representations of the best block's ID and its parent ID, in that order.
pub struct RedisClient {
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

/// Note that the two functions that write/modify the Redis store are `append_block` and
/// `take_best_block`. Both functions mutate the store within Redis transactions, and we also claim
/// that these are atomic, meaning they never leave the data store in an inconsistent state.
#[async_trait(?Send)]
impl ChainCache for RedisClient {
    /// Note that all Redis commands used in this function occur entirely in a single transaction,
    /// so this operation is atomic.
    async fn append_block(&mut self, block: Block) {
        let mut conn = self.pool.get().await.unwrap();
        let block_id_hex = String::from(block.id.0.clone());
        let parent_id_hex = String::from(block.parent_id.0.clone());
        let parent_id_key = concat_string!(block_id_hex, ":p");
        let block_height_key = concat_string!(block_id_hex, ":h");
        let block_transactions_key = concat_string!(block_id_hex, ":t");

        let mut pipe = redis::pipe();
        pipe
            // Make a transaction
            .atomic()
            // SET {block_id_hex}:p {parent_id_hex}
            .cmd("SET")
            .arg(&parent_id_key)
            .arg(&parent_id_hex)
            .ignore()
            // SET {block_id_hex}:h {block.height}
            .cmd("SET")
            .arg(&block_height_key)
            .arg(block.height)
            .ignore();

        for tx in &block.transactions {
            let tx_json = serde_json::to_string(&tx).unwrap();
            let tx_id = String::from(tx.id().0);

            // SADD {block_id_hex}:t {tx_id}
            pipe.cmd("SADD")
                .arg(&block_transactions_key)
                .arg(&tx_id)
                .ignore()
                // SET {tx_id} {tx_json}
                .cmd("SET")
                .arg(&tx_id)
                .arg(&tx_json)
                .ignore();
        }

        let _: () = pipe
            // DEL best_block
            .cmd("DEL")
            .arg(BEST_BLOCK)
            .ignore()
            // RPUSH best_block {block_id_hex} {parent_id_hex}
            .cmd("RPUSH")
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

    /// This function takes the current best block and replaces it with its parent block (if it
    /// exists in the cache).  Note: This function is atomic.
    ///
    /// ## Why it's atomic
    /// This function issue Redis commands in the following form:
    /// ```text
    ///   WATCH {BEST_BLOCK}
    ///   ... <Read information from existing best block in Redis> ... (1)
    ///   MULTI
    ///   ... <Redis commands that make up this transaction> ...
    ///   EXEC
    /// ```
    ///
    /// Recall that the transaction is made up of all commands starting from MULTI to EXEC. The WATCH
    /// command makes the transaction conditional on no change to BEST_BLOCK. i.e. the transaction will
    /// only execute as long as there was no modification of `BEST_BLOCK` between the WATCH and EXEC
    /// commands.
    ///
    /// The WATCH command is crucial because the reads made before the transaction at (1) could be
    /// out-of-date due to a completed `append_block` call. Note that it's sufficient to just watch
    /// `BEST_BLOCK` because `append_block` performs all mutations related to the best block in a
    /// single Redis transaction.
    async fn take_best_block(&mut self) -> Option<Block> {
        let mut conn = self.pool.get().await.unwrap();

        loop {
            let _: () = cmd("WATCH").arg(BEST_BLOCK).query_async(&mut conn).await.unwrap();

            if let Some(BlockRecord { id, height }) = self.get_best_block().await {
                let block_id_hex = String::from(id.0.clone());
                let block_id_digest32 = Digest32::try_from(block_id_hex.clone()).unwrap();
                let block_id = BlockId(block_id_digest32);

                let transaction_ids: Vec<String> = cmd("SMEMBERS")
                    .arg(concat_string!(block_id_hex, ":t"))
                    .query_async(&mut conn)
                    .await
                    .unwrap();

                let transactions_json: Vec<String> = cmd("MGET")
                    .arg(&transaction_ids)
                    .query_async(&mut conn)
                    .await
                    .unwrap();
                let transactions: Vec<Transaction> = transactions_json
                    .into_iter()
                    .map(|tx_json| serde_json::from_str(&tx_json).unwrap())
                    .collect();

                let parent_id_hex: String = cmd("GET")
                    .arg(concat_string!(block_id_hex, ":p"))
                    .query_async(&mut conn)
                    .await
                    .unwrap();
                let parent_digest32 = Digest32::try_from(parent_id_hex.clone()).unwrap();
                let parent_id = BlockId(parent_digest32);

                let mut pipe = redis::pipe();
                pipe.atomic() // Make this a transaction
                    .cmd("DEL")
                    .arg(BEST_BLOCK)
                    .arg(concat_string!(block_id_hex, ":t"))
                    .arg(&transaction_ids)
                    .ignore();

                // The new best block will now be the parent of the old best block, if the parent
                // exists in the cache.
                if let Ok(new_parent_id_hex) = cmd("GET")
                    .arg(concat_string!(parent_id_hex, ":p"))
                    .query_async::<_, String>(&mut conn)
                    .await
                {
                    // RPUSH best_block {parent_id_hex} {new_parent_id_hex}
                    pipe.cmd("RPUSH")
                        .arg(BEST_BLOCK)
                        .arg(&parent_id_hex)
                        .arg(&new_parent_id_hex);
                }

                let res = pipe.cmd("DEL").arg(&transaction_ids).query_async(&mut conn).await;

                match res {
                    Ok(()) => {
                        return Some(Block {
                            id: block_id,
                            parent_id,
                            height,
                            timestamp: 0, // todo: DEV-573
                            transactions,
                        });
                    }
                    Err(e) => {
                        if e.kind() == redis::ErrorKind::ExecAbortError {
                            // `BEST_BLOCK` was modified between the WATCH and EXEC statements, a
                            // data race. Let's try again.
                            continue;
                        } else {
                            panic!("Unexpected redis error {}", e);
                        }
                    }
                }
            } else {
                return None;
            }
        }
    }
}
