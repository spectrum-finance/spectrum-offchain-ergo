use std::sync::Arc;

use async_trait::async_trait;
use ergo_lib::{chain::transaction::TxId, ergo_chain_types::BlockId};
use rocksdb::WriteBatchWithTransaction;
use tokio::task::spawn_blocking;

use crate::model::{Block, BlockRecord};

use super::chain_cache::ChainCache;

static BEST_BLOCK: &str = "best_block";

pub struct RocksDBClient {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

/// The Rocksdb bindings are not async, so we must wrap any uses of the library in
/// `tokio::task::spawn_blocking`.
///
/// Note in this implementation that we often use both `bincode` and `serde_json` in
/// (de)serialization. We need to do this because `Transaction` from `ergo-lib` provides only JSON
/// (de)serialization. So we first serialize a transaction to a JSON string, and serialize the
/// String into binary with `bincode`. Ugly, but it works.
#[async_trait(?Send)]
impl ChainCache for RocksDBClient {
    async fn append_block(&mut self, block: Block) {
        let db = self.db.clone();

        // We need to do the processing here since we can't send `block.transactions` across threads.
        let tx_ids: Vec<TxId> = block.transactions.iter().map(|t| t.id()).collect();
        let serialized_transactions: Vec<_> = block
            .transactions
            .iter()
            .map(|t| bincode::serialize(&serde_json::to_string(t).unwrap()).unwrap())
            .collect();
        let best_block = BlockRecord {
            id: block.id.clone(),
            height: block.height,
        };

        spawn_blocking(move || {
            let mut batch = WriteBatchWithTransaction::<true>::default();
            batch.put(
                &block_id_parent_bytes(&block.id),
                bincode::serialize(&block.parent_id).unwrap(),
            );
            batch.put(
                &block_id_height_bytes(&block.id),
                bincode::serialize(&block.height).unwrap(),
            );

            // We package together all transactions ids into a Vec.
            batch.put(
                &block_id_transaction_bytes(&block.id),
                bincode::serialize(&tx_ids).unwrap(),
            );

            // Map each transaction id to a bincode-encoded representation of its transaction.
            for (tx_id, tx) in tx_ids.into_iter().zip(serialized_transactions) {
                batch.put(bincode::serialize(&tx_id).unwrap(), tx);
            }

            batch.put(
                bincode::serialize(BEST_BLOCK).unwrap(),
                bincode::serialize(&best_block).unwrap(),
            );

            assert!(db.write(batch).is_ok());
        })
        .await
        .unwrap();
    }

    async fn exists(&mut self, block_id: BlockId) -> bool {
        let db = self.db.clone();
        spawn_blocking(move || db.get(block_id_height_bytes(&block_id)).unwrap().is_some())
            .await
            .unwrap()
    }

    async fn get_best_block(&mut self) -> Option<BlockRecord> {
        let db = self.db.clone();
        spawn_blocking(move || {
            if let Ok(Some(bytes)) = db.get(bincode::serialize(BEST_BLOCK).unwrap()) {
                bincode::deserialize(&bytes).ok()
            } else {
                None
            }
        })
        .await
        .unwrap()
    }

    async fn take_best_block(&mut self) -> Option<Block> {
        let db = self.db.clone();
        if let Some(block_bytes) = spawn_blocking::<_, Option<Vec<u8>>>(move || {
            let best_block_key = bincode::serialize(BEST_BLOCK).unwrap();

            loop {
                let tx = db.transaction();
                // The call to `get_for_update` is crucial; it plays an identical role as the WATCH
                // command in redis (refer to docs of `take_best_block` in impl of [`RedisClient`].
                if let Some(best_block_bytes) = tx.get_for_update(&best_block_key, true).unwrap() {
                    let BlockRecord { id, height } = bincode::deserialize(&best_block_bytes).unwrap();

                    if let Some(tx_ids_bytes) = tx.get(&block_id_transaction_bytes(&id)).unwrap() {
                        let mut transactions = vec![];
                        let tx_ids: Vec<TxId> = bincode::deserialize(&tx_ids_bytes).unwrap();
                        for tx_id in tx_ids {
                            let tx_key = bincode::serialize(&tx_id).unwrap();
                            let tx_bytes = tx.get(&tx_key).unwrap().unwrap();
                            let block_tx_json_str: String = bincode::deserialize(&tx_bytes).unwrap();

                            // Don't need transaction anymore, delete
                            tx.delete(&tx_key).unwrap();

                            transactions.push(serde_json::from_str(&block_tx_json_str).unwrap());
                        }

                        let parent_id_bytes = tx.get(&block_id_parent_bytes(&id)).unwrap().unwrap();
                        let parent_id: BlockId = bincode::deserialize(&parent_id_bytes).unwrap();

                        tx.delete(&best_block_key).unwrap();

                        // The new best block will now be the parent of the old best block, if the parent
                        // exists in the cache.
                        if tx.get(&block_id_parent_bytes(&parent_id)).unwrap().is_some() {
                            let parent_id_height_bytes =
                                tx.get(&block_id_height_bytes(&parent_id)).unwrap().unwrap();
                            let parent_id_height: u32 =
                                bincode::deserialize(&parent_id_height_bytes).unwrap();

                            tx.put(
                                &best_block_key,
                                bincode::serialize(&BlockRecord {
                                    id: parent_id.clone(),
                                    height: parent_id_height,
                                })
                                .unwrap(),
                            )
                            .unwrap();
                        }
                        match tx.commit() {
                            Ok(_) => {
                                // Note: we can't send `Block` across the boundary since `Transaction` can't be
                                // sent between threads.
                                let best_block_bytes = bincode::serialize(
                                    &serde_json::to_string(&Block {
                                        id,
                                        parent_id,
                                        height,
                                        timestamp: 0, // todo: DEV-573
                                        transactions,
                                    })
                                    .unwrap(),
                                )
                                .unwrap();
                                return Some(best_block_bytes);
                            }
                            Err(e) => {
                                if e.kind() == rocksdb::ErrorKind::Busy {
                                    continue;
                                } else {
                                    panic!("Unexpected error: {}", e);
                                }
                            }
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
        })
        .await
        .unwrap()
        {
            let best_block_json_str: String = bincode::deserialize(&block_bytes).unwrap();
            let best_block = serde_json::from_str(&best_block_json_str).unwrap();
            Some(best_block)
        } else {
            None
        }
    }
}

fn block_id_parent_bytes(block_id: &BlockId) -> Vec<u8> {
    append_block_id_bytes(block_id, ":p")
}

fn block_id_height_bytes(block_id: &BlockId) -> Vec<u8> {
    append_block_id_bytes(block_id, ":h")
}

fn block_id_transaction_bytes(block_id: &BlockId) -> Vec<u8> {
    append_block_id_bytes(block_id, ":t")
}

fn append_block_id_bytes(block_id: &BlockId, s: &str) -> Vec<u8> {
    let mut bytes = bincode::serialize(block_id).unwrap();
    let p_bytes = bincode::serialize(s).unwrap();
    bytes.extend_from_slice(&p_bytes);
    bytes
}
