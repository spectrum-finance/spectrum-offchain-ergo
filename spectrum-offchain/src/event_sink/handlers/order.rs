use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use futures::{Sink, SinkExt};
use log::trace;
use tokio::sync::Mutex;

use ergo_mempool_sync::MempoolUpdate;

use crate::backlog::Backlog;
use crate::data::order::{OrderUpdate, PendingOrder};
use crate::data::{Has, OnChainOrder};
use crate::event_sink::handlers::types::TryFromBox;
use crate::event_sink::types::EventHandler;
use crate::event_source::data::LedgerTxEvent;

pub struct OrderUpdatesHandler<TSink, TOrd, TOrdProto, TBacklog> {
    pub topic: TSink,
    pub backlog: Arc<Mutex<TBacklog>>,
    pub order_lifespan: Duration,
    pub pd: PhantomData<TOrd>,
    pub pd_proto: PhantomData<TOrdProto>,
}

impl<TSink, TOrd, TOrdProto, TBacklog> OrderUpdatesHandler<TSink, TOrd, TOrdProto, TBacklog> {
    pub fn new(topic: TSink, backlog: Arc<Mutex<TBacklog>>, order_lifespan: Duration) -> Self {
        Self {
            topic,
            backlog,
            order_lifespan,
            pd: Default::default(),
            pd_proto: Default::default(),
        }
    }
}

#[async_trait(?Send)]
impl<TSink, TOrd, TOrdProto, TBacklog> EventHandler<LedgerTxEvent>
    for OrderUpdatesHandler<TSink, TOrd, TOrdProto, TBacklog>
where
    TSink: Sink<OrderUpdate<TOrdProto, TOrd::TOrderId>> + Unpin,
    TOrd: OnChainOrder,
    TOrdProto: Has<TOrd::TOrderId> + TryFromBox,
    TOrd::TOrderId: From<BoxId> + Copy,
    TBacklog: Backlog<TOrd>,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        let res = match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for i in tx.clone().inputs {
                    let order_id = TOrd::TOrderId::from(i.box_id);
                    if self.backlog.lock().await.exists(order_id).await {
                        is_success = true;
                        let _ = self.topic.feed(OrderUpdate::OrderEliminated(order_id)).await;
                    }
                }
                let ts_now = Utc::now().timestamp();
                if ts_now - timestamp <= self.order_lifespan.num_milliseconds() {
                    for bx in &tx.outputs {
                        if let Some(order) = TOrdProto::try_from_box(bx.clone()) {
                            is_success = true;
                            let _ = self
                                .topic
                                .feed(OrderUpdate::NewOrder(PendingOrder {
                                    order,
                                    timestamp: Utc::now().timestamp(),
                                }))
                                .await;
                        }
                    }
                }
                if is_success {
                    trace!(target: "offchain_lm", "Observing new order");
                    return None;
                }
                Some(LedgerTxEvent::AppliedTx { tx, timestamp })
            }
            LedgerTxEvent::UnappliedTx(tx) => {
                let mut is_success = false;
                for bx in &tx.outputs {
                    if let Some(order) = TOrdProto::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self
                            .topic
                            .feed(OrderUpdate::OrderEliminated(
                                <TOrdProto as Has<TOrd::TOrderId>>::get::<TOrd::TOrderId>(&order),
                            ))
                            .await;
                    }
                }
                if is_success {
                    trace!(target: "offchain_lm", "Known order is eliminated");
                    return None;
                }
                Some(LedgerTxEvent::UnappliedTx(tx))
            }
        };
        let _ = self.topic.flush().await;
        res
    }
}

#[async_trait(?Send)]
impl<TSink, TOrd, TOrdProto, TBacklog> EventHandler<MempoolUpdate>
    for OrderUpdatesHandler<TSink, TOrd, TOrdProto, TBacklog>
where
    TSink: Sink<OrderUpdate<TOrd, TOrd::TOrderId>> + Unpin,
    TOrd: OnChainOrder + TryFromBox,
    TOrd::TOrderId: From<BoxId> + Copy,
    TBacklog: Backlog<TOrd>,
{
    async fn try_handle(&mut self, ev: MempoolUpdate) -> Option<MempoolUpdate> {
        let res = match ev {
            MempoolUpdate::TxAccepted(tx) => {
                let mut is_success = false;
                for i in tx.clone().inputs {
                    let order_id = TOrd::TOrderId::from(i.box_id);
                    if self.backlog.lock().await.exists(order_id).await {
                        is_success = true;
                        let _ = self.topic.feed(OrderUpdate::OrderEliminated(order_id)).await;
                    }
                }
                for bx in &tx.outputs {
                    if let Some(order) = TOrd::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self
                            .topic
                            .feed(OrderUpdate::NewOrder(PendingOrder {
                                order,
                                timestamp: Utc::now().timestamp(),
                            }))
                            .await;
                    }
                }
                if is_success {
                    trace!(target: "offchain_lm", "Observing new order in mempool");
                    return None;
                }
                Some(MempoolUpdate::TxAccepted(tx))
            }
            MempoolUpdate::TxWithdrawn(tx) => {
                let mut is_success = false;
                for bx in &tx.outputs {
                    if let Some(order) = TOrd::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self
                            .topic
                            .feed(OrderUpdate::OrderEliminated(order.get_self_ref()))
                            .await;
                    }
                }
                if is_success {
                    trace!(target: "offchain_lm", "Known order is eliminated in mempool");
                    return None;
                }
                Some(MempoolUpdate::TxWithdrawn(tx))
            }
            ev => Some(ev),
        };
        let _ = self.topic.flush().await;
        res
    }
}
