use futures::stream::StreamExt;
use futures::{stream, Stream};

use ergo_chain_sync::ChainUpgrade;

use crate::event_source::data::LedgerTxEvent;

pub mod data;

pub fn event_source_ledger<S>(upstream: S) -> impl Stream<Item = LedgerTxEvent>
where
    S: Stream<Item = ChainUpgrade>,
{
    upstream.flat_map(|u| stream::iter(process_upgrade(u)))
}

fn process_upgrade(upgr: ChainUpgrade) -> Vec<LedgerTxEvent> {
    match upgr {
        ChainUpgrade::RollForward(blk) => {
            let ts = blk.timestamp;
            blk.transactions
                .into_iter()
                .map(|tx| LedgerTxEvent::AppliedTx {
                    tx,
                    timestamp: ts as i64,
                    height: blk.height,
                })
                .collect()
        }
        ChainUpgrade::RollBackward(blk) => blk
            .transactions
            .into_iter()
            .rev() // we unapply txs in reverse order.
            .map(LedgerTxEvent::UnappliedTx)
            .collect(),
    }
}
