use std::sync::Arc;
use std::time::Duration;

use bounded_integer::BoundedU8;
use ergo_lib::ergotree_ir::chain::address::Address;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::Constant;
use ergo_lib::ergotree_ir::mir::expr::Expr;
use futures::channel::mpsc;
use isahc::{prelude::*, HttpClient};
use parking_lot::Mutex;

use ergo_chain_sync::cache::chain_cache::InMemoryCache;
use ergo_chain_sync::client::node::ErgoNodeHttpClient;
use ergo_chain_sync::client::types::Url;
use ergo_chain_sync::ChainSync;
use spectrum_offchain::backlog::persistence::BacklogStoreRocksDB;
use spectrum_offchain::backlog::{BacklogConfig, BacklogService};
use spectrum_offchain::box_resolver::rocksdb::EntityRepoRocksDB;
use spectrum_offchain::data::order::OrderUpdate;
use spectrum_offchain::data::unique_entity::{Confirmed, StateUpdate};
use spectrum_offchain::event_sink::handlers::entity::ConfirmedUpdateHandler;
use spectrum_offchain::event_sink::handlers::order::OrderUpdatesHandler;
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::event_sink::types::{EventHandler, NoopDefaultHandler};
use spectrum_offchain::event_source::data::LedgerTxEvent;
use spectrum_offchain::event_source::event_source_ledger;
use spectrum_offchain::rocksdb::RocksConfig;

use crate::bundle::rocksdb::BundleRepoRocksDB;
use crate::data::bundle::IndexedStakingBundle;
use crate::data::funding::{ExecutorWallet, FundingUpdate};
use crate::data::order::Order;
use crate::data::pool::Pool;
use crate::data::AsBox;
use crate::event_sink::handlers::bundle::ConfirmedBundleUpdateHadler;
use crate::event_sink::handlers::funding::ConfirmedFundingHadler;
use crate::executor::OrderExecutor;
use crate::funding::FundingRepoRocksDB;
use crate::program::rocksdb::ProgramRepoRocksDB;
use crate::prover::NoopProver;

pub mod bundle;
pub mod data;
pub mod ergo;
pub mod event_sink;
pub mod executor;
pub mod funding;
pub mod program;
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
    let chain_sync = ChainSync::init(500000, node.clone(), cache).await;

    let backlog_store = BacklogStoreRocksDB::new(RocksConfig {
        db_path: format!("./tmp/backlog"),
    });
    let backlog_conf = BacklogConfig {
        order_lifespan: chrono::Duration::seconds(60 * 60 * 24),
        order_exec_time: chrono::Duration::seconds(60 * 60 * 24),
        retry_suspended_prob: <BoundedU8<0, 100>>::new(20).unwrap(),
    };
    let backlog = Arc::new(Mutex::new(BacklogService::new::<Order>(
        backlog_store,
        backlog_conf,
    )));
    let pools = Arc::new(Mutex::new(EntityRepoRocksDB::new(RocksConfig {
        db_path: format!("./tmp/pools"),
    })));
    let programs = Arc::new(Mutex::new(ProgramRepoRocksDB::new(RocksConfig {
        db_path: format!("./tmp/programs"),
    })));
    let bundles = Arc::new(Mutex::new(BundleRepoRocksDB::new(RocksConfig {
        db_path: format!("./tmp/bundles"),
    })));
    let funding = Arc::new(Mutex::new(FundingRepoRocksDB::new(RocksConfig {
        db_path: format!("./tmp/funding"),
    })));
    let prover = NoopProver;
    let executor_prop = ErgoTree::try_from(Expr::Const(Constant::from(true))).unwrap();
    let executor = OrderExecutor::new(
        node.clone(),
        Arc::clone(&backlog),
        Arc::clone(&pools),
        Arc::clone(&bundles),
        Arc::clone(&funding),
        prover,
        executor_prop,
    );

    let default_handler = NoopDefaultHandler;
    // hadnler:
    // pools
    let (pool_snd, pool_recv) = mpsc::unbounded::<Confirmed<StateUpdate<Pool>>>();
    let pool_han = ConfirmedUpdateHandler::<_, Pool, _>::new(pool_snd, Arc::clone(&pools));
    // orders
    let (order_snd, pool_recv) = mpsc::unbounded::<OrderUpdate<Order>>();
    let order_han = OrderUpdatesHandler::<_, Order, _>::new(
        order_snd,
        Arc::clone(&backlog),
        chrono::Duration::seconds(60 * 60 * 24),
    );
    // bundles
    let (bundle_snd, bundle_recv) = mpsc::unbounded::<Confirmed<StateUpdate<AsBox<IndexedStakingBundle>>>>();
    let bundle_han = ConfirmedBundleUpdateHadler {
        topic: bundle_snd,
        bundles: Arc::clone(&bundles),
        programs: Arc::clone(&programs),
    };
    // funding
    let (funding_snd, funding_recv) = mpsc::unbounded::<Confirmed<FundingUpdate>>();
    let funding_han = ConfirmedFundingHadler {
        topic: funding_snd,
        repo: Arc::clone(&funding),
        wallet: ExecutorWallet::from(Address::P2SH([0u8; 24])),
    };
    let handlers: Vec<Box<dyn EventHandler<LedgerTxEvent>>> = vec![
        Box::new(pool_han),
        Box::new(order_han),
        Box::new(bundle_han),
        Box::new(funding_han),
    ];

    let event_source = event_source_ledger(chain_sync);
    let event_sink = process_events(event_source, handlers, default_handler);
}
