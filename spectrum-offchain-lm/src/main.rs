use std::sync::Arc;
use std::time::Duration;

use bounded_integer::BoundedU8;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::Constant;
use ergo_lib::ergotree_ir::mir::expr::Expr;
use isahc::{HttpClient, prelude::*};
use parking_lot::Mutex;

use ergo_chain_sync::cache::chain_cache::InMemoryCache;
use ergo_chain_sync::ChainSync;
use ergo_chain_sync::client::node::ErgoNodeHttpClient;
use ergo_chain_sync::client::types::Url;
use spectrum_offchain::backlog::{BacklogConfig, BacklogService};
use spectrum_offchain::backlog::persistence::BacklogStoreRocksDB;
use spectrum_offchain::box_resolver::rocksdb::EntityRepoRocksDB;
use spectrum_offchain::rocksdb::RocksConfig;

use crate::bundle::rocksdb::BundleRepoRocksDB;
use crate::data::order::Order;
use crate::executor::OrderExecutor;
use crate::funding::FundingRepoRocksDB;
use crate::prover::NoopProver;

pub mod bundle;
pub mod data;
pub mod ergo;
pub mod event_sink;
pub mod executor;
pub mod funding;
pub mod pool;
pub mod prover;
pub mod scheduler;
pub mod validators;

#[tokio::main]
async fn main() {
    log4rs::init_file("conf/log4rs.yaml", Default::default()).unwrap();

    let client = HttpClient::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let node = ErgoNodeHttpClient::new(client, Url::from("http://213.239.193.208:9053"));
    let cache = InMemoryCache::new();
    let mut _chain_sync = ChainSync::init(500000, node.clone(), cache).await;

    let backlog_store = BacklogStoreRocksDB::new(RocksConfig {
        db_path: format!("./tmp/backlog"),
    });
    let backlog_conf = BacklogConfig {
        order_lifespan: chrono::Duration::seconds(60 * 60 * 24),
        order_exec_time: chrono::Duration::seconds(60 * 60 * 24),
        retry_suspended_prob: <BoundedU8<0, 100>>::new(20).unwrap(),
    };
    let backlog = Arc::new(Mutex::new(BacklogService::new::<Order>(backlog_store, backlog_conf)));
    let pools = Arc::new(Mutex::new(EntityRepoRocksDB::new(RocksConfig {
        db_path: format!("./tmp/pools"),
    })));
    let bundles = Arc::new(Mutex::new(BundleRepoRocksDB::new(RocksConfig {
        db_path: format!("./tmp/bundles"),
    })));
    let funding = Arc::new(Mutex::new(FundingRepoRocksDB::new(RocksConfig {
        db_path: format!("./tmp/funding"),
    })));
    let prover = NoopProver;
    let executor_prop =  ErgoTree::try_from(Expr::Const(Constant::from(true))).unwrap();
    let executor = OrderExecutor::new(node.clone(), backlog, pools, bundles, funding, prover, executor_prop);
}
