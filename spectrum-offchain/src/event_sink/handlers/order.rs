use std::marker::PhantomData;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use tokio::sync::mpsc::UnboundedSender;

use crate::backlog::data::BacklogOrder;
use crate::backlog::persistence::BacklogStore;
use crate::data::order::{EliminatedOrder, PendingOrder};
use crate::data::OnChainOrder;
use crate::event_sink::handlers::types::TryFromBox;
use crate::event_sink::types::EventHandler;
use crate::event_source::data::LedgerTxEvent;

pub struct PendingOrdersHandler<TOrd, P> {
    topic: UnboundedSender<PendingOrder<TOrd>>,
    order_lifespan: Duration,
    parser: P,
}

#[async_trait(?Send)]
impl<TOrd, P> EventHandler<LedgerTxEvent> for PendingOrdersHandler<TOrd, P>
where
    P: TryFromBox<TOrd>,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let ts_now = Utc::now().timestamp();
                if ts_now - timestamp <= self.order_lifespan.num_milliseconds() {
                    let mut is_success = false;
                    for bx in &tx.outputs {
                        if let Some(order) = self.parser.try_from(bx.clone()) {
                            is_success = true;
                            let _ = self.topic.send(PendingOrder {
                                order,
                                timestamp: Utc::now().timestamp(),
                            });
                        }
                    }
                    if is_success {
                        return None;
                    }
                }
                Some(LedgerTxEvent::AppliedTx { tx, timestamp })
            }
            ev => Some(ev),
        }
    }
}

pub struct EliminatedOrdersHandler<TOrd, TOrdId, TStore> {
    topic: UnboundedSender<EliminatedOrder<TOrdId>>,
    store: TStore,
    pd: PhantomData<TOrd>,
}

#[async_trait(?Send)]
impl<TOrd, TOrdId, TStore> EventHandler<LedgerTxEvent> for EliminatedOrdersHandler<TOrd, TOrdId, TStore>
where
    TOrdId: From<BoxId> + Clone,
    TOrd: OnChainOrder<TOrderId = TOrdId>,
    TStore: BacklogStore<TOrd>,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for i in tx.clone().inputs {
                    let ord_id = TOrdId::from(i.box_id);
                    if let Some(BacklogOrder { timestamp, .. }) = self.store.get(ord_id.clone()).await {
                        is_success = true;
                        let _ = self.topic.send(EliminatedOrder {
                            order_id: ord_id,
                            timestamp,
                        });
                    }
                }
                if is_success {
                    return None;
                }
                Some(LedgerTxEvent::AppliedTx { tx, timestamp })
            }
            ev => Some(ev),
        }
    }
}
