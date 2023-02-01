use std::cell::{RefCell, RefMut};
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use ergo_lib::chain::transaction::{Transaction, TxId};
use futures::stream::FusedStream;
use futures::Stream;
use wasm_timer::Delay;

use ergo_chain_sync::model::Block;
use ergo_chain_sync::{ChainUpgrade, InitChainSync};

use crate::client::node::ErgoNetwork;

pub mod client;

#[derive(Debug, Clone)]
pub enum MempoolUpdate {
    /// Tx was accepted to mempool.
    TxAccepted(Transaction),
    /// Tx was discarded.
    TxWithdrawn(Transaction),
}

#[derive(Debug, Clone)]
struct SyncState {
    latest_blocks: VecDeque<HashSet<TxId>>,
    mempool_projection: HashMap<TxId, Transaction>,
    pending_updates: VecDeque<MempoolUpdate>,
}

impl SyncState {
    fn empty() -> Self {
        Self {
            latest_blocks: VecDeque::new(),
            mempool_projection: HashMap::new(),
            pending_updates: VecDeque::new(),
        }
    }
}

const KEEP_LAST_BLOCKS: usize = 10;

impl SyncState {
    fn push_block(&mut self, blk: Block) {
        self.latest_blocks
            .push_back(HashSet::from_iter(blk.transactions.into_iter().map(|tx| tx.id())));
        if self.latest_blocks.len() > KEEP_LAST_BLOCKS {
            self.latest_blocks.pop_front();
        }
    }

    fn pop_block(&mut self) {
        self.latest_blocks.pop_back();
    }
}

pub struct MempoolSyncConf {
    pub sync_interval: Delay,
}

#[pin_project::pin_project]
pub struct MempoolSync<'a, TClient, TChainSync> {
    conf: MempoolSyncConf,
    client: &'a TClient,
    #[pin]
    chain_sync: TChainSync,
    state: Rc<RefCell<SyncState>>,
}

impl<'a, TClient, TChainSync> MempoolSync<'a, TClient, TChainSync>
where
    TClient: ErgoNetwork,
{
    pub async fn init<TChainSyncMaker: InitChainSync<TChainSync>>(
        conf: MempoolSyncConf,
        client: &'a TClient,
        chain_sync_maker: TChainSyncMaker,
    ) -> MempoolSync<'a, TClient, TChainSync> {
        let chain_tip_height = client.get_best_height().await;
        let start_at = chain_tip_height as usize - KEEP_LAST_BLOCKS;
        let chain_sync = chain_sync_maker.init(start_at as u32, None).await;
        Self {
            conf,
            client,
            chain_sync,
            state: Rc::new(RefCell::new(SyncState::empty())),
        }
    }
}

const TXS_PER_REQUEST: usize = 100;

#[allow(clippy::await_holding_refcell_ref)]
async fn sync<'a, TClient: ErgoNetwork>(client: &TClient, mut state: RefMut<'a, SyncState>) {
    let mut pool: Vec<Transaction> = Vec::new();
    let mut offset = 0;
    loop {
        let mut txs = client.fetch_mempool(offset, TXS_PER_REQUEST).await;
        let num_txs = txs.len();
        pool.append(&mut txs);
        if num_txs < TXS_PER_REQUEST {
            break;
        }
        offset += num_txs;
    }
    let new_pool_ids = pool.iter().map(|tx| tx.id()).collect::<HashSet<_>>();
    let old_pool_ids = state.mempool_projection.keys().cloned().collect::<HashSet<_>>();
    let elim_txs = old_pool_ids.difference(&new_pool_ids);
    'check_withdrawn: for tx_id in elim_txs {
        state.mempool_projection.remove(tx_id);
        for blk in state.latest_blocks.iter() {
            if blk.contains(tx_id) {
                continue 'check_withdrawn;
            }
        }
        if let Some(tx) = state.mempool_projection.get(tx_id).cloned() {
            state.pending_updates.push_back(MempoolUpdate::TxWithdrawn(tx));
        }
    }
    for tx in pool {
        if state.mempool_projection.contains_key(&tx.id()) {
            continue;
        }
        state.mempool_projection.insert(tx.id(), tx.clone());
        state.pending_updates.push_back(MempoolUpdate::TxAccepted(tx));
    }
}

impl<'a, TClient, TChainSync> Stream for MempoolSync<'a, TClient, TChainSync>
where
    TClient: ErgoNetwork,
    TChainSync: Stream<Item = ChainUpgrade> + Unpin,
{
    type Item = MempoolUpdate;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        let client: &TClient = this.client;
        if Future::poll(Pin::new(&mut this.conf.sync_interval), cx).is_ready() {
            let mut state = this.state.borrow_mut();
            // Sync chain tail
            loop {
                match Stream::poll_next(this.chain_sync.as_mut(), cx) {
                    Poll::Ready(Some(ChainUpgrade::RollForward(blk))) => state.push_block(blk),
                    Poll::Ready(Some(ChainUpgrade::RollBackward(_))) => state.pop_block(),
                    Poll::Ready(None) => {}
                    Poll::Pending => break,
                }
            }
            let mut sync_fut = Box::pin(sync(client, state));
            // Drive sync to completion
            loop {
                match sync_fut.as_mut().poll(cx) {
                    Poll::Ready(_) => break,
                    Poll::Pending => continue,
                }
            }
        }
        if let Some(upgr) = this.state.borrow_mut().pending_updates.pop_front() {
            return Poll::Ready(Some(upgr));
        }
        Poll::Pending
    }
}

impl<'a, TClient, TChainSync> FusedStream for MempoolSync<'a, TClient, TChainSync>
where
    MempoolSync<'a, TClient, TChainSync>: Stream,
{
    /// MempoolSync stream is never terminated.
    fn is_terminated(&self) -> bool {
        false
    }
}
