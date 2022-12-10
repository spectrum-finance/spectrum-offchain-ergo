use std::sync::Arc;

use chrono::Utc;
use ergo_chain_sync::{
    cache::{
        chain_cache::{ChainCache, InMemoryCache},
        redis::RedisClient,
        rocksdb::RocksDBClient,
    },
    model::Block,
};
use ergo_lib::{
    chain::transaction::Transaction,
    ergo_chain_types::{BlockId, Digest32},
};
use sigma_test_util::force_any_val;

#[async_std::test]
async fn test_redis() {
    let mut client = RedisClient::new("redis://127.0.0.1/");
    test_client(client).await;
}

#[tokio::test]
async fn test_rocksdb() {
    test_client(RocksDBClient {
        db: Arc::new(rocksdb::OptimisticTransactionDB::open_default("./tmp").unwrap()),
    })
    .await;
}

#[async_std::test]
async fn test_inmemory_cache() {
    test_client(InMemoryCache::new()).await;
}

/// Generate a chain of 30 `BlockId`s, representing blocks that each contain 10 transactions. We
/// add them to the cache and remove them via `take_best_block`.
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
        let timestamp = Utc::now().timestamp() as u64;
        let block = Block {
            id: id.clone(),
            parent_id,
            height,
            timestamp,
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

        // Check that the collections of transactions coincide.
        assert_eq!(b0.transactions.len(), b1.transactions.len());
        for tx0 in b0.transactions {
            let tx1 = b1.transactions.iter().find(|t| tx0.id() == t.id()).unwrap();
            assert_eq!(tx0, *tx1);
        }
    }
}
