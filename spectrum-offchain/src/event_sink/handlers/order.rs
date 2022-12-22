use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;

use ergo_mempool_sync::MempoolUpdate;

use crate::backlog::Backlog;
use crate::data::order::{OrderUpdate, PendingOrder};
use crate::data::OnChainOrder;
use crate::event_sink::handlers::types::TryFromBox;
use crate::event_sink::types::EventHandler;
use crate::event_source::data::LedgerTxEvent;

pub struct OrderUpdatesHandler<TOrd: OnChainOrder, TBacklog> {
    topic: UnboundedSender<OrderUpdate<TOrd>>,
    backlog: Arc<Mutex<TBacklog>>,
    order_lifespan: Duration,
}

#[async_trait(?Send)]
impl<TOrd, TBacklog> EventHandler<LedgerTxEvent> for OrderUpdatesHandler<TOrd, TBacklog>
where
    TOrd: OnChainOrder + TryFromBox,
    TOrd::TOrderId: From<BoxId> + Copy,
    TBacklog: Backlog<TOrd>,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for i in tx.clone().inputs {
                    let order_id = TOrd::TOrderId::from(i.box_id);
                    if self.backlog.lock().exists(order_id).await {
                        is_success = true;
                        let _ = self.topic.send(OrderUpdate::OrderEliminated(order_id));
                    }
                }
                let ts_now = Utc::now().timestamp();
                if ts_now - timestamp <= self.order_lifespan.num_milliseconds() {
                    for bx in &tx.outputs {
                        if let Some(order) = TOrd::try_from_box(bx.clone()) {
                            is_success = true;
                            let _ = self.topic.send(OrderUpdate::NewOrder(PendingOrder {
                                order,
                                timestamp: Utc::now().timestamp(),
                            }));
                        }
                    }
                }
                if is_success {
                    return None;
                }
                Some(LedgerTxEvent::AppliedTx { tx, timestamp })
            }
            LedgerTxEvent::UnappliedTx(tx) => {
                let mut is_success = false;
                for bx in &tx.outputs {
                    if let Some(order) = TOrd::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self
                            .topic
                            .send(OrderUpdate::OrderEliminated(order.get_self_ref()));
                    }
                }
                if is_success {
                    return None;
                }
                Some(LedgerTxEvent::UnappliedTx(tx))
            }
        }
    }
}

#[async_trait(?Send)]
impl<TOrd, TBacklog> EventHandler<MempoolUpdate> for OrderUpdatesHandler<TOrd, TBacklog>
where
    TOrd: OnChainOrder + TryFromBox,
    TOrd::TOrderId: From<BoxId> + Copy,
    TBacklog: Backlog<TOrd>,
{
    async fn try_handle(&mut self, ev: MempoolUpdate) -> Option<MempoolUpdate> {
        match ev {
            MempoolUpdate::TxAccepted(tx) => {
                let mut is_success = false;
                for i in tx.clone().inputs {
                    let order_id = TOrd::TOrderId::from(i.box_id);
                    if self.backlog.lock().exists(order_id).await {
                        is_success = true;
                        let _ = self.topic.send(OrderUpdate::OrderEliminated(order_id));
                    }
                }
                for bx in &tx.outputs {
                    if let Some(order) = TOrd::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self.topic.send(OrderUpdate::NewOrder(PendingOrder {
                            order,
                            timestamp: Utc::now().timestamp(),
                        }));
                    }
                }
                if is_success {
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
                            .send(OrderUpdate::OrderEliminated(order.get_self_ref()));
                    }
                }
                if is_success {
                    return None;
                }
                Some(MempoolUpdate::TxWithdrawn(tx))
            }
        }
    }
}
