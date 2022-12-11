use std::sync::Arc;

use async_trait::async_trait;
use ergo_lib::{
    chain::transaction::{Transaction, TxId},
    ergo_chain_types::BlockId,
    ergotree_ir::serialization::SigmaSerializable,
};
use rocksdb::WriteBatchWithTransaction;
use serde::{Deserialize, Serialize};
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

        let block_send = BlockSend::from(block);

        spawn_blocking(move || {
            let mut batch = WriteBatchWithTransaction::<true>::default();
            batch.put(
                &block_id_parent_bytes(&block_send.id),
                bincode::serialize(&block_send.parent_id).unwrap(),
            );
            batch.put(
                &block_id_height_bytes(&block_send.id),
                bincode::serialize(&block_send.height).unwrap(),
            );

            let serialized_transactions = block_send.transactions_in_bytes.clone();
            let block = Block::from(block_send);

            let tx_ids: Vec<TxId> = block.transactions.iter().map(|t| t.id()).collect();
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
                bincode::serialize(&BlockRecord {
                    id: block.id,
                    height: block.height,
                })
                .unwrap(),
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
        spawn_blocking::<_, Option<BlockSend>>(move || {
            let best_block_key = bincode::serialize(BEST_BLOCK).unwrap();

            loop {
                let tx = db.transaction();
                // The call to `get_for_update` is crucial; it plays an identical role as the WATCH
                // command in redis (refer to docs of `take_best_block` in impl of [`RedisClient`].
                if let Some(best_block_bytes) = tx.get_for_update(&best_block_key, true).unwrap() {
                    let BlockRecord { id, height } = bincode::deserialize(&best_block_bytes).unwrap();

                    if let Some(tx_ids_bytes) = tx.get(&block_id_transaction_bytes(&id)).unwrap() {
                        let mut transactions_in_bytes = vec![];
                        let tx_ids: Vec<TxId> = bincode::deserialize(&tx_ids_bytes).unwrap();
                        for tx_id in tx_ids {
                            let tx_key = bincode::serialize(&tx_id).unwrap();
                            let tx_bytes = tx.get(&tx_key).unwrap().unwrap();

                            // Don't need transaction anymore, delete
                            tx.delete(&tx_key).unwrap();

                            transactions_in_bytes.push(tx_bytes);
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
                                return Some(BlockSend {
                                    id,
                                    parent_id,
                                    height,
                                    timestamp: 0, // todo: DEV-573
                                    transactions_in_bytes,
                                });
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
        .map(Block::from)
    }
}

/// A transformation of `Block` with Ergo-serialized byte representation for transactions. We use
/// this type to efficiently get `Block` between thread boundaries, since `Block` doesn't impl
/// `Send` since `Transaction` doesn't.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BlockSend {
    pub id: BlockId,
    pub parent_id: BlockId,
    pub height: u32,
    pub timestamp: u64,
    pub transactions_in_bytes: Vec<Vec<u8>>,
}

impl From<BlockSend> for Block {
    fn from(b: BlockSend) -> Self {
        Self {
            id: b.id,
            parent_id: b.parent_id,
            height: b.height,
            timestamp: b.timestamp,
            transactions: b
                .transactions_in_bytes
                .into_iter()
                .map(|b| Transaction::sigma_parse_bytes(&b).unwrap())
                .collect(),
        }
    }
}

impl From<Block> for BlockSend {
    fn from(b: Block) -> Self {
        Self {
            id: b.id,
            parent_id: b.parent_id,
            height: b.height,
            timestamp: b.timestamp,
            transactions_in_bytes: b
                .transactions
                .into_iter()
                .map(|t| t.sigma_serialize_bytes().unwrap())
                .collect(),
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
