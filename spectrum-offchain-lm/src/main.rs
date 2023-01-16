use std::sync::{Arc, Once};

use clap::{arg, Parser};
use ergo_lib::ergotree_ir::chain::address::{AddressEncoder, NetworkPrefix};
use futures::channel::mpsc;
use futures::stream::select_all;
use futures::StreamExt;
use isahc::{prelude::*, HttpClient};
use log::info;
use serde::Deserialize;
use tokio::sync::Mutex;

use ergo_chain_sync::cache::rocksdb::ChainCacheRocksDB;
use ergo_chain_sync::client::node::ErgoNodeHttpClient;
use ergo_chain_sync::client::types::Url;
use ergo_chain_sync::rocksdb::RocksConfig;
use ergo_chain_sync::ChainSync;
use spectrum_offchain::backlog::persistence::BacklogStoreRocksDB;
use spectrum_offchain::backlog::process::backlog_stream;
use spectrum_offchain::backlog::{BacklogConfig, BacklogService, BacklogTracing};
use spectrum_offchain::box_resolver::persistence::EntityRepoTracing;
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
use crate::bundle::BundleRepoTracing;
use crate::data::bundle::IndexedStakingBundle;
use crate::data::funding::{ExecutorWallet, FundingUpdate};
use crate::data::order::Order;
use crate::data::pool::Pool;
use crate::data::AsBox;
use crate::event_sink::handlers::bundle::ConfirmedBundleUpdateHadler;
use crate::event_sink::handlers::funding::ConfirmedFundingHadler;
use crate::event_sink::handlers::program::ConfirmedProgramUpdateHandler;
use crate::event_sink::handlers::schedule::ConfirmedScheduleUpdateHandler;
use crate::executor::OrderExecutor;
use crate::funding::process::funding_update_stream;
use crate::funding::{FundingRepoRocksDB, FundingRepoTracing};
use crate::program::rocksdb::ProgramRepoRocksDB;
use crate::prover::{SeedPhrase, Wallet};
use crate::scheduler::process::distribution_stream;
use crate::scheduler::{ScheduleRepoRocksDB, ScheduleRepoTracing};
use crate::streaming::boxed;

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
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_yaml_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_yaml::from_str(&raw_config).expect("Invalid configuration file");

    if let Some(log4rs_path) = args.log4rs_path {
        log4rs::init_file(log4rs_path, Default::default()).unwrap();
    } else {
        log4rs::init_file(config.log4rs_yaml_path, Default::default()).unwrap();
    }

    let client = HttpClient::builder()
        .timeout(std::time::Duration::from_secs(
            config.http_client_timeout_duration_secs as u64,
        ))
        .build()
        .unwrap();

    let node = ErgoNodeHttpClient::new(client, config.node_addr);
    let cache = ChainCacheRocksDB::new(RocksConfig {
        db_path: config.chain_cache_db_path.into(),
    });
    let signal_tip_reached: Once = Once::new();
    let chain_sync = ChainSync::init(
        config.chain_sync_starting_height,
        &node,
        cache,
        Some(&signal_tip_reached),
    )
    .await;

    let backlog_store = BacklogStoreRocksDB::new(RocksConfig {
        db_path: config.backlog_store_db_path.into(),
    });
    let backlog = Arc::new(Mutex::new(BacklogTracing::wrap(BacklogService::new::<Order>(
        backlog_store,
        config.backlog_config.clone(),
    ))));
    let pools = Arc::new(Mutex::new(EntityRepoTracing::wrap(EntityRepoRocksDB::new(
        RocksConfig {
            db_path: config.entity_repo_db_path.into(),
        },
    ))));
    let programs = Arc::new(Mutex::new(ProgramRepoRocksDB::new(RocksConfig {
        db_path: config.program_repo_db_path.into(),
    })));
    let bundles = Arc::new(Mutex::new(BundleRepoTracing::wrap(BundleRepoRocksDB::new(
        RocksConfig {
            db_path: config.bundle_repo_db_path.into(),
        },
    ))));
    let funding = Arc::new(Mutex::new(FundingRepoTracing::wrap(FundingRepoRocksDB::new(
        RocksConfig {
            db_path: config.funding_repo_db_path.into(),
        },
    ))));
    let (prover, funding_addr) = Wallet::try_from_seed(config.operator_funding_secret).expect("Invalid seed");

    info!(
        "Funding address is {}",
        AddressEncoder::encode_address_as_string(NetworkPrefix::Mainnet, &funding_addr)
    );

    let executor = OrderExecutor::new(
        &node,
        Arc::clone(&backlog),
        Arc::clone(&pools),
        Arc::clone(&bundles),
        Arc::clone(&funding),
        prover,
        config.operator_reward_addr.ergo_tree(),
    );
    let executor_stream = boxed(executor_stream(executor, &signal_tip_reached));

    let default_handler = NoopDefaultHandler;

    // pools
    let (pool_snd, pool_recv) = mpsc::unbounded::<Confirmed<StateUpdate<AsBox<Pool>>>>();
    let pool_han = ConfirmedUpdateHandler::<_, AsBox<Pool>, _>::new(pool_snd, Arc::clone(&pools));
    let pool_update_stream = boxed(entity_tracking_stream(pool_recv, pools));

    // orders
    let (order_snd, order_recv) = mpsc::unbounded::<OrderUpdate<Order>>();
    let order_han = OrderUpdatesHandler::<_, Order, _>::new(
        order_snd,
        Arc::clone(&backlog),
        config.backlog_config.order_lifespan,
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
        wallet: funding_addr.into(),
    };
    let funding_update_stream = boxed(funding_update_stream(funding_recv, Arc::clone(&funding)));

    let schedules = Arc::new(Mutex::new(ScheduleRepoTracing::wrap(ScheduleRepoRocksDB::new(
        RocksConfig {
            db_path: config.schedule_repo_db_path.into(),
        },
    ))));
    let schedule_han = ConfirmedScheduleUpdateHandler {
        schedules: Arc::clone(&schedules),
    };
    let scheduler_stream = boxed(distribution_stream(
        backlog,
        schedules,
        bundles,
        &node,
        20,
        std::time::Duration::from_secs(60),
        &signal_tip_reached,
    ));

    let program_han = ConfirmedProgramUpdateHandler {
        programs: Arc::clone(&programs),
    };

    let handlers: Vec<Box<dyn EventHandler<LedgerTxEvent>>> = vec![
        Box::new(pool_han),
        Box::new(order_han),
        Box::new(bundle_han),
        Box::new(funding_han),
        Box::new(schedule_han),
        Box::new(program_han),
    ];

    let event_source = event_source_ledger(chain_sync);
    let process_events_stream = boxed(process_events(event_source, handlers, default_handler));

    let mut app = select_all(vec![
        process_events_stream,
        executor_stream,
        pool_update_stream,
        backlog_stream,
        bundle_update_stream,
        funding_update_stream,
        scheduler_stream,
    ]);

    loop {
        app.select_next_some().await;
    }
}

#[derive(Deserialize)]
struct AppConfig<'a> {
    node_addr: Url,
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
    chain_cache_db_path: &'a str,
    operator_reward_addr: ExecutorWallet,
    operator_funding_secret: SeedPhrase,
}

#[derive(Parser)]
#[command(name = "spectrum-offchain-lm")]
#[command(author = "Ilya Oskin (@oskin1), Timothy Ling (@kettlebell) for Spectrum Finance")]
#[command(version = "0.1")]
#[command(about = "Spectrum Finance Liquidity Mining Reference Node", long_about = None)]
struct AppArgs {
    /// Path to the YAML configuration file.
    #[arg(long, short)]
    config_yaml_path: String,
    /// Optional path to the log4rs YAML configuration file. NOTE: overrides path specified in config YAML file.
    #[arg(long, short)]
    log4rs_path: Option<String>,
}
