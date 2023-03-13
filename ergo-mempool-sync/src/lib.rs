use std::cell::{Cell, RefCell, RefMut};
use std::collections::{HashMap, HashSet, VecDeque};

use std::pin::Pin;
use std::rc::Rc;

use std::time::Duration;

use futures_timer::Delay;
use log::trace;

use async_stream::stream;
use ergo_lib::chain::transaction::{Transaction, TxId};
use futures::stream::select;
use futures::{Stream, StreamExt};
use log::info;

use futures::future::ready;

use ergo_chain_sync::model::Block;
use ergo_chain_sync::{ChainSync, ChainUpgrade, InitChainSync};

use crate::client::node::ErgoNetwork;

pub mod client;

#[derive(Debug, Clone)]
pub enum MempoolUpdate {
    /// Tx was accepted to mempool.
    TxAccepted(Transaction),
    /// Tx was discarded.
    TxWithdrawn(Transaction),
    /// Tx was confirmed.
    TxConfirmed(Transaction),
}

#[derive(Debug, Clone)]
pub struct SyncState {
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
    pub sync_interval: u64,
}

#[pin_project::pin_project]
pub struct MempoolSync<'a, TClient> {
    conf: MempoolSyncConf,
    client: &'a TClient,
    #[pin]
    chain_sync: Pin<Box<dyn Stream<Item = ChainUpgrade> + 'a>>,
    state: Rc<RefCell<SyncState>>,
    #[pin]
    delay: Cell<Option<Delay>>,
}

impl<'a, TClient> MempoolSync<'a, TClient>
where
    TClient: ErgoNetwork,
{
    pub async fn init<
        TChainSyncClient: 'a,
        TCache,
        TChainSyncMaker: InitChainSync<'a, ChainSync<'a, TChainSyncClient, TCache>>,
    >(
        conf: MempoolSyncConf,
        client: &'a TClient,
        chain_sync_maker: TChainSyncMaker,
    ) -> MempoolSync<'a, TClient> {
        let chain_tip_height = client.get_best_height().await;
        let start_at = match chain_tip_height {
            Ok(height) => height as usize - KEEP_LAST_BLOCKS,
            Err(error) => {
                info!(
                    target: "mempool_sync",
                    "# Failed to request best height: {}",
                    error,
                );
                0
            }
        };
        let chain_sync = chain_sync_maker.init(start_at as u32, None).await;
        Self {
            conf,
            client,
            chain_sync,
            state: Rc::new(RefCell::new(SyncState::empty())),
            delay: Cell::new(None),
        }
    }
}

const TXS_PER_REQUEST: usize = 100;

#[allow(clippy::await_holding_refcell_ref)]
async fn sync<'a, TClient: ErgoNetwork>(client: &TClient, mut state: RefMut<'a, SyncState>) {
    let mut pool: Vec<Transaction> = Vec::new();
    let mut offset = 0;
    loop {
        let txs = client.fetch_mempool(offset, TXS_PER_REQUEST).await;
        match txs {
            Ok(mut mempool_txs) => {
                let num_txs = mempool_txs.len();
                pool.append(mempool_txs.as_mut());
                if num_txs < TXS_PER_REQUEST {
                    break;
                }
                offset += num_txs
            }
            Err(error) => {
                info!(
                    target: "mempool_sync",
                    "# Failed to request next mempool transactions: {}",
                    error,
                );
            }
        }
    }
    let new_pool_ids = pool.iter().map(|tx| tx.id()).collect::<HashSet<_>>();
    let old_pool_ids = state.mempool_projection.keys().cloned().collect::<HashSet<_>>();
    let elim_txs = old_pool_ids.difference(&new_pool_ids);
    'check_withdrawn: for tx_id in elim_txs {
        if let Some(tx) = state.mempool_projection.remove(tx_id) {
            for blk in state.latest_blocks.iter() {
                if blk.contains(tx_id) {
                    state.pending_updates.push_back(MempoolUpdate::TxConfirmed(tx));
                    continue 'check_withdrawn;
                }
            }
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

pub enum Combined {
    Mempool(MempoolUpdate),
    Empty,
}

pub fn mempool_sync_stream_combined<'a, TClient>(
    mempool_sync: MempoolSync<'a, TClient>,
) -> impl Stream<Item = Combined> + 'a
where
    TClient: ErgoNetwork + Unpin,
{
    select(
        chain_sync_for_mempool_sync_stream(mempool_sync.chain_sync, mempool_sync.state.clone()),
        mempool_sync_stream(
            mempool_sync.state.clone(),
            mempool_sync.client,
            mempool_sync.delay,
            mempool_sync.conf,
        ),
    )
}

pub fn mempool_sync_stream<'a, TClient>(
    state_rc: Rc<RefCell<SyncState>>,
    client: &'a TClient,
    delay: Cell<Option<Delay>>,
    conf: MempoolSyncConf,
) -> impl Stream<Item = Combined> + 'a
where
    TClient: ErgoNetwork + Unpin,
{
    stream! {
        loop {
            let state = state_rc.borrow_mut();
            let client: &TClient = client;
            if let Some(delay) = delay.take() {
                delay.await;
            }
            sync(client, state).await;
            if let Some(upgr) = state_rc.borrow_mut().pending_updates.pop_front() {
                yield Combined::Mempool(upgr);
            } else {
                delay.set(Some(Delay::new(Duration::from_secs(conf.sync_interval))))
            }
        }
    }
}

pub fn chain_sync_for_mempool_sync_stream<'a, S>(
    upstream: S,
    state_rc: Rc<RefCell<SyncState>>,
) -> impl Stream<Item = Combined> + 'a
where
    S: Stream<Item = ChainUpgrade> + 'a,
{
    upstream.then(move |event| {
        let mut state = state_rc.borrow_mut();
        match event {
            ChainUpgrade::RollForward(blk) => state.push_block(blk),
            ChainUpgrade::RollBackward(_) => state.pop_block(),
        };

        ready(Combined::Empty)
    })
}
