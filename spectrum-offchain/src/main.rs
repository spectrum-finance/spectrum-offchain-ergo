use std::time::Duration;

use futures::prelude::*;
use isahc::{prelude::*, HttpClient};

use ergo_chain_sync::cache::chain_cache::InMemoryCache;
use ergo_chain_sync::client::node::ErgoNodeHttpClient;
use ergo_chain_sync::client::types::Url;
use ergo_chain_sync::{ChainSync, ChainUpgrade};

pub mod box_resolver;
pub mod data;
pub mod event_sink;
pub mod event_source;
pub mod backlog;
pub mod executor;
pub mod network;

#[tokio::main]
async fn main() {
    log4rs::init_file("conf/log4rs.yaml", Default::default()).unwrap();

    let client = HttpClient::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let node = ErgoNodeHttpClient::new(client, Url::from("http://213.239.193.208:9053"));
    let cache = InMemoryCache::new();
    let mut chain_sync = ChainSync::init(500000, node, cache).await;

    println!("Initialized");
    loop {
        match chain_sync.select_next_some().await {
            ChainUpgrade::RollForward(blk) => {
                println!("RollFwd({:?})", blk.id)
            }
            ChainUpgrade::RollBackward(blk) => {
                println!("RollBwd({:?})", blk.id)
            }
        }
    }
}
