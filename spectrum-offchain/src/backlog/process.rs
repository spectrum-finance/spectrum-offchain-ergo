use std::sync::Arc;

use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use crate::backlog::Backlog;
use crate::data::order::OrderUpdate;
use crate::data::OnChainOrder;

/// Create backlog stream that drives processing of order events.
pub fn backlog_stream<'a, S, TOrd, TBacklog>(backlog: TBacklog, upstream: S) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = OrderUpdate<TOrd>> + 'a,
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
