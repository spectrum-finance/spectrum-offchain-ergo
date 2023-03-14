use std::sync::Arc;

use futures::{Stream, StreamExt};
use log::trace;
use spectrum_offchain::{
    backlog::Backlog,
    data::order::{OrderUpdate, PendingOrder},
};
use tokio::sync::Mutex;

use crate::{
    bundle::BundleRepo,
    data::{
        order::{Order, OrderProto},
        AsBox, BundleId, OrderId,
    },
};

/// Create backlog stream that drives processing of order events.
pub fn backlog_stream<'a, S, TBacklog, TBundles>(
    backlog: Arc<Mutex<TBacklog>>,
    bundle_repo: Arc<Mutex<TBundles>>,
    upstream: S,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = OrderUpdate<OrderProto, OrderId>> + 'a,
    TBacklog: Backlog<Order> + 'a,
    TBundles: BundleRepo + 'a,
{
    trace!(target: "offchain_lm", "Watching for Backlog events..");
    upstream.then(move |upd| {
        let backlog = Arc::clone(&backlog);
        let bundle_repo = bundle_repo.clone();
        async move {
            let mut backlog = backlog.lock().await;
            match upd {
                OrderUpdate::NewOrder(pending_order) => {
                    let order = match pending_order.order {
                        OrderProto::Deposit(d) => Order::Deposit(d),
                        OrderProto::Redeem(r) => {
                            let bundle_repo = bundle_repo.lock().await;
                            let bundle_id = BundleId::from(r.1.bundle_key.token_id);
                            trace!(target: "offchain_lm", "Requesting bundle with id [{:?}]", bundle_id);
                            let b = bundle_repo.get_last_confirmed(bundle_id).await.unwrap();
                            let pool_id = b.0 .1.pool_id;
                            let redeem = r.1.finalize(pool_id);
                            Order::Redeem(AsBox(r.0, redeem))
                        }
                        OrderProto::Compound(c) => Order::Compound(c),
                    };
                    let pending_order = PendingOrder {
                        order,
                        timestamp: pending_order.timestamp,
                    };
                    backlog.put(pending_order).await;
                }
                OrderUpdate::OrderEliminated(elim_oid) => backlog.remove(elim_oid).await,
            }
        }
    })
}
