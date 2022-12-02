use std::pin::Pin;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use futures::stream::select_all;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use crate::backlog::Backlog;
use crate::data::order::{EliminatedOrder, PendingOrder};
use crate::data::OnChainOrder;

/// Create backlog stream that drives processing of order events.
pub fn backlog_stream<'a, TOrd, TBacklog>(
    backlog: TBacklog,
    pending_orders: UnboundedReceiver<PendingOrder<TOrd>>,
    elim_orders: UnboundedReceiver<EliminatedOrder<TOrd::TOrderId>>,
) -> impl Stream<Item = ()> + 'a
where
    TOrd: OnChainOrder + 'a,
    TBacklog: Backlog<TOrd> + 'a,
{
    let backlog = Arc::new(Mutex::new(backlog));
    select_all(vec![
        track_pending_orders(backlog.clone(), pending_orders),
        track_elim_orders(backlog, elim_orders),
    ])
}

fn track_pending_orders<'a, TOrd, TBacklog>(
    backlog: Arc<Mutex<TBacklog>>,
    upstream: UnboundedReceiver<PendingOrder<TOrd>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TOrd: OnChainOrder + 'a,
    TBacklog: Backlog<TOrd> + 'a,
{
    Box::pin(upstream.then(move |ord| {
        let backlog = Arc::clone(&backlog);
        async move {
            let mut backlog_guard = backlog.lock();
            backlog_guard.put(ord).await;
        }
    }))
}

fn track_elim_orders<'a, TOrd, TBacklog>(
    backlog: Arc<Mutex<TBacklog>>,
    upstream: UnboundedReceiver<EliminatedOrder<TOrd::TOrderId>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TOrd: OnChainOrder + 'a,
    TBacklog: Backlog<TOrd> + 'a,
{
    Box::pin(upstream.then(move |ord| {
        let backlog = Arc::clone(&backlog);
        async move {
            let mut backlog_guard = backlog.lock();
            backlog_guard.remove(ord.order_id).await;
        }
    }))
}
