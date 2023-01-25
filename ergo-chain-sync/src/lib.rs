use std::cell::{Cell, RefCell};
use std::cmp::max;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Once;
use std::task::{Context, Poll};
use std::{thread, time};
use std::time::Duration;
use futures::executor::block_on;

use futures::stream::FusedStream;
use futures::Stream;
use futures_timer::Delay;
use log::trace;
use pin_project::pin_project;

use crate::cache::chain_cache::ChainCache;
use crate::client::node::ErgoNetwork;
use crate::model::Block;

pub mod cache;
pub mod client;
pub mod model;
pub mod rocksdb;

#[derive(Debug, Clone)]
pub enum ChainUpgrade {
    RollForward(Block),
    RollBackward(Block),
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

#[async_trait::async_trait(?Send)]
pub trait InitChainSync<TChainSync> {
    async fn init(self, starting_height: u32, tip_reached_signal: Option<&'static Once>) -> TChainSync;
}

pub struct ChainSyncNonInit<'a, TClient, TCache> {
    client: &'a TClient,
    cache: TCache,
}

impl<'a, TClient, TCache> ChainSyncNonInit<'a, TClient, TCache> {
    pub fn new(client: &'a TClient, cache: TCache) -> Self {
        Self { client, cache }
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, TClient, TCache> InitChainSync<ChainSync<'a, TClient, TCache>>
    for ChainSyncNonInit<'a, TClient, TCache>
where
    TClient: ErgoNetwork,
    TCache: ChainCache,
{
    async fn init(
        self,
        starting_height: u32,
        tip_reached_signal: Option<&'a Once>,
    ) -> ChainSync<'a, TClient, TCache> {
        ChainSync::init(starting_height, self.client, self.cache, tip_reached_signal).await
    }
}

#[pin_project]
pub struct ChainSync<'a, TClient, TCache> {
    starting_height: u32,
    client: &'a TClient,
    cache: Rc<RefCell<TCache>>,
    state: Rc<RefCell<SyncState>>,
    #[pin]
    delay: Cell<Option<Delay>>,
    tip_reached_signal: Option<&'a Once>,
}

impl<'a, TClient, TCache> ChainSync<'a, TClient, TCache>
where
    TClient: ErgoNetwork,
    TCache: ChainCache,
{
    pub async fn init(
        starting_height: u32,
        client: &'a TClient,
        mut cache: TCache,
        tip_reached_signal: Option<&'a Once>,
    ) -> ChainSync<'a, TClient, TCache> {
        let best_block = cache.get_best_block().await;
        let start_at = if let Some(best_block) = best_block {
            trace!(target: "chain_sync", "Best block is [{}]", best_block.id);
            max(best_block.height, starting_height)
        } else {
            starting_height
        };
        Self {
            starting_height,
            client,
            cache: Rc::new(RefCell::new(cache)),
            state: Rc::new(RefCell::new(SyncState {
                next_height: start_at,
            })),
            delay: Cell::new(None),
            tip_reached_signal,
        }
    }

    /// Try acquiring next upgrade from the network.
    /// `None` is returned when no upgrade is available at the moment.
    async fn try_upgrade(&self) -> Option<ChainUpgrade> {
        let next_height = { self.state.borrow().next_height };
        trace!(target: "chain_sync", "Processing height [{}]", next_height);
        if let Some(api_blk) = self.client.get_block_at(next_height).await {
            trace!(
                target: "chain_sync",
                "Processing block [{:?}] at height [{}]",
                api_blk.header.id,
                next_height
            );
            let parent_id = api_blk.header.parent_id;
            let mut cache = self.cache.borrow_mut();
            let linked = cache.exists(parent_id).await;
            if linked || api_blk.header.height == self.starting_height {
                trace!(target: "chain_sync", "Chain is linked, upgrading ..");
                let blk = Block::from(api_blk);
                let ten_millis = time::Duration::from_millis(1000);
                // thread::sleep(ten_millis);
                trace!(target: "chain_sync", "cache.append_block(blk.clone()).await;");
                block_on(cache.append_block(blk.clone()));
                trace!(target: "chain_sync", "Chain is linked, finish upgrade.");
                self.state.borrow_mut().upgrade();
                trace!(target: "chain_sync", "self.state.borrow_mut().upgrade();");
                return Some(ChainUpgrade::RollForward(blk));
            } else {
                // Local chain does not link anymore
                trace!(target: "chain_sync", "Chain does not link, downgrading ..");
                if let Some(discarded_blk) = cache.take_best_block().await {
                    self.state.borrow_mut().downgrade();
                    return Some(ChainUpgrade::RollBackward(discarded_blk));
                }
            }
        }
        None
    }
}

const THROTTLE_SECS: u64 = 1;

impl<'a, TClient, TCache> Stream for ChainSync<'a, TClient, TCache>
where
    TClient: ErgoNetwork + Unpin,
    TCache: ChainCache + Unpin,
{
    type Item = ChainUpgrade;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        if let Some(mut delay) = self.delay.take() {
            match Future::poll(Pin::new(&mut delay), cx) {
                Poll::Ready(_) => {}
                Poll::Pending => {
                    self.delay.set(Some(delay));
                    return Poll::Pending;
                }
            }
        }

        let mut upgr_fut = Box::pin(self.try_upgrade());
        loop {
            match upgr_fut.as_mut().poll(cx) {
                Poll::Ready(Some(upgr)) => return Poll::Ready(Some(upgr)),
                Poll::Ready(None) => {
                    self.delay
                        .set(Some(Delay::new(Duration::from_secs(THROTTLE_SECS))));
                    if let Some(sig) = self.tip_reached_signal {
                        sig.call_once(|| {
                            trace!(target: "chain_sync", "Tip reached, waiting for new blocks ..");
                        });
                    }
                    return Poll::Pending;
                }
                Poll::Pending => continue,
            }
        }
    }
}

impl<'a, TClient, TCache> FusedStream for ChainSync<'a, TClient, TCache>
where
    ChainSync<'a, TClient, TCache>: Stream,
{
    /// ChainSync stream is never terminated.
    fn is_terminated(&self) -> bool {
        false
    }
}
