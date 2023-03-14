use std::sync::Arc;

use futures::{Stream, StreamExt};
use log::trace;
use spectrum_offchain::data::order::{OrderUpdate, PendingOrder};
use tokio::sync::Mutex;

use crate::{
    bundle::BundleRepo,
    data::{
        order::{Order, OrderProto},
        AsBox, BundleId, OrderId,
    },
};

/// Convert stream of `OrderUpdate<OrderProto, OrderId>>` to stream of
/// `OrderUpdate<OrderProto, OrderId>>` for backlog stream.
pub fn convert_order_proto<'a, S, TBundles>(
    bundle_repo: Arc<Mutex<TBundles>>,
    upstream: S,
) -> impl Stream<Item = OrderUpdate<Order, OrderId>> + 'a
where
    S: Stream<Item = OrderUpdate<OrderProto, OrderId>> + 'a,
    TBundles: BundleRepo + 'a,
{
    upstream.then(move |upd| {
        let bundle_repo = bundle_repo.clone();
        async move {
            match upd {
                OrderUpdate::NewOrder(pending_order) => {
                    let order = match pending_order.order {
                        OrderProto::Deposit(d) => Order::Deposit(d),
                        OrderProto::Redeem(r) => {
                            let bundle_repo = bundle_repo.lock().await;
                            let bundle_id = BundleId::from(r.1.bundle_key.token_id);
                            trace!(target: "offchain_lm", "Requesting bundle with id [{:?}]", bundle_id);
                            let bundle = bundle_repo.get_last_confirmed(bundle_id).await.unwrap();
                            let pool_id = bundle.0 .1.pool_id;
                            let redeem = r.1.finalize(pool_id);
                            Order::Redeem(AsBox(r.0, redeem))
                        }
                        OrderProto::Compound(c) => Order::Compound(c),
                    };
                    let pending_order = PendingOrder {
                        order,
                        timestamp: pending_order.timestamp,
                    };
                    OrderUpdate::NewOrder(pending_order)
                }
                OrderUpdate::OrderEliminated(elim_oid) => OrderUpdate::OrderEliminated(elim_oid),
            }
        }
    })
}
