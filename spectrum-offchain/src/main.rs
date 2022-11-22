use futures::prelude::*;

use ergo_chain_sync::cache::chain_cache::InMemoryCache;
use ergo_chain_sync::client::node::ErgoNodeHttpClient;
use ergo_chain_sync::client::types::Url;
use ergo_chain_sync::{ChainSync, ChainSyncConf, ChainUpgrade};

#[tokio::main]
async fn main() {
    let client = reqwest::Client::new();
    let node = ErgoNodeHttpClient::new(client, Url::from("http://213.239.193.208:9053"));
    let cache = InMemoryCache::new();
    let conf = ChainSyncConf {
        starting_height: 500000,
    };
    let mut chain_sync = ChainSync::init(conf, node, cache).await;

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
