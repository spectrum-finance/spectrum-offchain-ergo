use futures::stream::StreamExt;
use futures::{stream, Stream};

use ergo_chain_sync::ChainUpgrade;

use crate::event_source::data::TxEvent;

pub mod data;

pub fn event_source<S>(upstream: S) -> impl Stream<Item = TxEvent>
where
    S: Stream<Item = ChainUpgrade>,
{
    upstream.flat_map(|u| stream::iter(process_upgrade(u)))
}

fn process_upgrade(upgr: ChainUpgrade) -> Vec<TxEvent> {
    match upgr {
        ChainUpgrade::RollForward(blk) => blk.transactions.into_iter().map(TxEvent::AppliedTx).collect(),
        ChainUpgrade::RollBackward(blk) => blk
            .transactions
            .into_iter()
            .rev() // we unapply txs in reverse order.
            .map(TxEvent::UnappliedTx)
            .collect(),
    }
}
