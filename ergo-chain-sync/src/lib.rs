use std::cell::RefCell;
use std::cmp::max;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use futures::stream::FusedStream;
use futures::Stream;
use log::trace;

use crate::cache::chain_cache::ChainCache;
use crate::client::node::ErgoNetwork;
use crate::model::Block;

pub mod cache;
pub mod client;
pub mod model;

#[derive(Debug, Clone)]
pub enum ChainUpgrade {
    RollForward(Block),
    RollBackward(Block),
}

pub struct ChainSyncConf {
    pub starting_height: u32,
}

#[derive(Debug, Clone)]
struct SyncState {
    next_height: u32,
}

impl SyncState {
    fn upgrade(&mut self) {
        self.next_height += 1;
    }

    fn downgrade(&mut self) {
        self.next_height -= 1;
    }
}

pub struct ChainSync<TClient, TCache> {
    conf: ChainSyncConf,
    client: TClient,
    cache: TCache,
    state: Rc<RefCell<SyncState>>,
}

impl<TClient, TCache> ChainSync<TClient, TCache>
where
    TClient: ErgoNetwork,
    TCache: ChainCache,
{
    pub async fn init(conf: ChainSyncConf, client: TClient, mut cache: TCache) -> Self {
        let best_block = cache.get_best_block().await;
        let start_at = if let Some(best_block) = best_block {
            max(best_block.height, conf.starting_height)
        } else {
            conf.starting_height
        };
        Self {
            conf,
            client,
            cache,
            state: Rc::new(RefCell::new(SyncState {
                next_height: start_at,
            })),
        }
    }

    /// Try acquiring next upgrade from the network.
    /// `None` is returned when no upgrade is available at the moment.
    async fn try_upgrade(&mut self) -> Option<ChainUpgrade> {
        let mut state = self.state.borrow_mut();
        trace!("Processing height [{}]", state.next_height);
        if let Some(api_blk) = self.client.get_block_at(state.next_height).await {
            trace!(
                "Processing block [{:?}] at height [{}]",
                api_blk.header.id.clone(),
                state.next_height
            );
            let parent_id = api_blk.header.parent_id.clone();
            let linked = self.cache.exists(parent_id.clone()).await;
            if linked || api_blk.header.height == self.conf.starting_height {
                trace!("Chain is linked, upgrading ..");
                let blk = Block::from(api_blk);
                self.cache.append_block(blk.clone()).await;
                state.upgrade();
                return Some(ChainUpgrade::RollForward(blk));
            } else {
                // Local chain does not link anymore
                trace!("Chain does not link, downgrading ..");
                if let Some(discarded_blk) = self.cache.take_best_block().await {
                    state.downgrade();
                    return Some(ChainUpgrade::RollBackward(discarded_blk));
                }
            }
        }
        None
    }
}

impl<TClient, TCache> Stream for ChainSync<TClient, TCache>
where
    TClient: ErgoNetwork + Unpin,
    TCache: ChainCache + Unpin,
{
    type Item = ChainUpgrade;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let mut upgr_fut = Box::pin(self.try_upgrade());
        loop {
            match upgr_fut.as_mut().poll(cx) {
                Poll::Ready(Some(upgr)) => return Poll::Ready(Some(upgr)),
                Poll::Ready(None) => return Poll::Pending,
                Poll::Pending => continue,
            }
        }
    }
}

impl<TClient, TCache> FusedStream for ChainSync<TClient, TCache>
where
    ChainSync<TClient, TCache>: Stream,
{
    /// ChainSync stream is never terminated.
    fn is_terminated(&self) -> bool {
        false
    }
}
