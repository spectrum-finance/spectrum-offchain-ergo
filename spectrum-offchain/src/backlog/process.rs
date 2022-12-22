use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use crate::backlog::Backlog;
use crate::data::order::OrderUpdate;
use crate::data::OnChainOrder;

/// Create backlog stream that drives processing of order events.
pub fn backlog_stream<'a, TOrd, TBacklog>(
    backlog: TBacklog,
    upstream: UnboundedReceiver<OrderUpdate<TOrd>>,
) -> impl Stream<Item = ()> + 'a
where
    TOrd: OnChainOrder + 'a,
    TBacklog: Backlog<TOrd> + 'a,
{
    let backlog = Arc::new(Mutex::new(backlog));
    upstream.then(move |upd| {
        let backlog = Arc::clone(&backlog);
        async move {
            match upd {
                OrderUpdate::NewOrder(pending_order) => backlog.lock().put(pending_order).await,
                OrderUpdate::OrderEliminated(elim_oid) => backlog.lock().remove(elim_oid).await,
            }
        }
    })
}
