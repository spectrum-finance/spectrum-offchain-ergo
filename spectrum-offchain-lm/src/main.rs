use ergo_chain_sync::cache::chain_cache::InMemoryCache;
use std::str::FromStr;
use std::sync::Arc;

use ergo_lib::ergotree_ir::chain::address::Address;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::Constant;
use ergo_lib::ergotree_ir::mir::expr::Expr;
use futures::channel::mpsc;
use futures::stream::select_all;
use futures::StreamExt;
use isahc::{prelude::*, HttpClient};
use tokio::sync::Mutex;

use ergo_chain_sync::client::node::ErgoNodeHttpClient;
use ergo_chain_sync::client::types::Url;
use ergo_chain_sync::rocksdb::RocksConfig;
use ergo_chain_sync::ChainSync;
use serde::{Deserialize, Serialize};
use spectrum_offchain::backlog::persistence::BacklogStoreRocksDB;
use spectrum_offchain::backlog::process::backlog_stream;
use spectrum_offchain::backlog::{BacklogConfig, BacklogService};
use spectrum_offchain::box_resolver::process::entity_tracking_stream;
use spectrum_offchain::box_resolver::rocksdb::EntityRepoRocksDB;
use spectrum_offchain::data::order::OrderUpdate;
use spectrum_offchain::data::unique_entity::{Confirmed, StateUpdate};
use spectrum_offchain::event_sink::handlers::entity::ConfirmedUpdateHandler;
use spectrum_offchain::event_sink::handlers::order::OrderUpdatesHandler;
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::event_sink::types::{EventHandler, NoopDefaultHandler};
use spectrum_offchain::event_source::data::LedgerTxEvent;
use spectrum_offchain::event_source::event_source_ledger;
use spectrum_offchain::executor::executor_stream;

use crate::bundle::process::bundle_update_stream;
use crate::bundle::rocksdb::BundleRepoRocksDB;
use crate::data::bundle::IndexedStakingBundle;
use crate::data::funding::{ExecutorWallet, FundingUpdate};
use crate::data::order::Order;
use crate::data::pool::Pool;
use crate::data::AsBox;
use crate::event_sink::handlers::bundle::ConfirmedBundleUpdateHadler;
use crate::event_sink::handlers::funding::ConfirmedFundingHadler;
use crate::executor::OrderExecutor;
use crate::funding::process::funding_update_stream;
use crate::funding::FundingRepoRocksDB;
use crate::program::process::track_programs;
use crate::program::rocksdb::ProgramRepoRocksDB;
use crate::prover::NoopProver;
use crate::scheduler::process::run_distribution_scheduler;
use crate::scheduler::ScheduleRepoRocksDB;
use crate::streaming::{boxed, AsSink};

pub mod bundle;
pub mod data;
pub mod ergo;
pub mod event_sink;
pub mod executor;
pub mod funding;
pub mod program;
pub mod prover;
pub mod scheduler;
mod streaming;
pub mod validators;

#[tokio::main]
async fn main() {
    let s = std::fs::read_to_string("lm_config.yml").unwrap();
    let config: LMConfig = serde_yaml::from_str(&s).unwrap();

    log4rs::init_file(config.log4rs_yaml_path, Default::default()).unwrap();

    let client = HttpClient::builder()
        .timeout(std::time::Duration::from_secs(
            config.http_client_timeout_duration_secs as u64,
        ))
        .build()
        .unwrap();

    let node = ErgoNodeHttpClient::new(client, Url::from_str(config.node_addr).unwrap());
    let cache = InMemoryCache::new();
    let chain_sync = ChainSync::init(config.chain_sync_starting_height, node.clone(), cache).await;

    let backlog_store = BacklogStoreRocksDB::new(RocksConfig {
        db_path: config.backlog_store_db_path.into(),
    });
    let backlog = Arc::new(Mutex::new(BacklogService::new::<Order>(
        backlog_store,
        config.backlog_config,
    )));
    let pools = Arc::new(Mutex::new(EntityRepoRocksDB::new(RocksConfig {
        db_path: config.entity_repo_db_path.into(),
    })));
    let programs = Arc::new(Mutex::new(ProgramRepoRocksDB::new(RocksConfig {
        db_path: config.program_repo_db_path.into(),
    })));
    let bundles = Arc::new(Mutex::new(BundleRepoRocksDB::new(RocksConfig {
        db_path: config.bundle_repo_db_path.into(),
    })));
    let funding = Arc::new(Mutex::new(FundingRepoRocksDB::new(RocksConfig {
        db_path: config.funding_repo_db_path.into(),
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
    let executor_stream = boxed(executor_stream(executor));

    let default_handler = NoopDefaultHandler;

    // pools
    let (pool_snd, pool_recv) = async_channel::unbounded::<Confirmed<StateUpdate<AsBox<Pool>>>>();
    let pool_han = ConfirmedUpdateHandler::<_, AsBox<Pool>, _>::new(AsSink(pool_snd), Arc::clone(&pools));
    let pool_update_stream = boxed(entity_tracking_stream(pool_recv.clone(), pools));

    // orders
    let (order_snd, order_recv) = mpsc::unbounded::<OrderUpdate<Order>>();
    let order_han = OrderUpdatesHandler::<_, Order, _>::new(
        order_snd,
        Arc::clone(&backlog),
        chrono::Duration::seconds(60 * 60 * 24),
    );
    let backlog_stream = boxed(backlog_stream(Arc::clone(&backlog), order_recv));

    // bundles
    let (bundle_snd, bundle_recv) = mpsc::unbounded::<Confirmed<StateUpdate<AsBox<IndexedStakingBundle>>>>();
    let bundle_han = ConfirmedBundleUpdateHadler {
        topic: bundle_snd,
        bundles: Arc::clone(&bundles),
        programs: Arc::clone(&programs),
    };
    let bundle_update_stream = boxed(bundle_update_stream(bundle_recv, Arc::clone(&bundles)));

    // funding
    let (funding_snd, funding_recv) = mpsc::unbounded::<Confirmed<FundingUpdate>>();
    let funding_han = ConfirmedFundingHadler {
        topic: funding_snd,
        repo: Arc::clone(&funding),
        wallet: ExecutorWallet::from(Address::P2SH([0u8; 24])),
    };
    let funding_update_stream = boxed(funding_update_stream(funding_recv, Arc::clone(&funding)));

    // program
    let program_update_stream = boxed(track_programs(pool_recv.clone(), Arc::clone(&programs)));

    let handlers: Vec<Box<dyn EventHandler<LedgerTxEvent>>> = vec![
        Box::new(pool_han),
        Box::new(order_han),
        Box::new(bundle_han),
        Box::new(funding_han),
    ];

    let event_source = event_source_ledger(chain_sync);
    let process_events_stream = boxed(process_events(event_source, handlers, default_handler));

    let schedules = Arc::new(Mutex::new(ScheduleRepoRocksDB::new(RocksConfig {
        db_path: config.schedule_repo_db_path.into(),
    })));
    let scheduer_stream = boxed(run_distribution_scheduler(
        pool_recv,
        backlog,
        schedules,
        bundles,
        node,
        20,
        std::time::Duration::from_secs(60),
    ));

    let mut app = select_all(vec![
        process_events_stream,
        executor_stream,
        pool_update_stream,
        backlog_stream,
        bundle_update_stream,
        funding_update_stream,
        program_update_stream,
        scheduer_stream,
    ]);

    loop {
        app.select_next_some().await;
    }
}

#[derive(Serialize, Deserialize)]
struct LMConfig<'a> {
    node_addr: &'a str,
    http_client_timeout_duration_secs: u32,
    chain_sync_starting_height: u32,
    backlog_config: BacklogConfig,
    log4rs_yaml_path: &'a str,
    backlog_store_db_path: &'a str,
    entity_repo_db_path: &'a str,
    program_repo_db_path: &'a str,
    bundle_repo_db_path: &'a str,
    funding_repo_db_path: &'a str,
    schedule_repo_db_path: &'a str,
}
