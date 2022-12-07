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

#[async_trait(?Send)]
impl ChainCache for RocksDBClient {
    async fn append_block(&mut self, block: Block) {
        let db = self.db.clone();

        // We need to do the processing here since we can't send `block.transactions` across threads.
        let tx_ids: Vec<TxId> = block.transactions.iter().map(|t| t.id()).collect();
        let serialized_transactions: Vec<_> = block
            .transactions
            .iter()
            .map(|t| bincode::serialize(t).unwrap())
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
        todo!()
    }

    async fn get_best_block(&mut self) -> Option<BlockRecord> {
        todo!()
    }

    async fn take_best_block(&mut self) -> Option<Block> {
        todo!()
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
