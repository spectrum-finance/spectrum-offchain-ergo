use chrono::Utc;
use tokio::sync::mpsc::UnboundedSender;

use crate::data::order::events::PendingOrder;
use crate::event_sink::handlers::types::TryFromBox;
use crate::event_sink::types::EventHandler;
use crate::event_source::data::LedgerTxEvent;

pub struct PendingOrderHandler<TOrd, P>
where
    P: TryFromBox<TOrd>,
{
    topic: UnboundedSender<PendingOrder<TOrd>>,
    parser: P,
}

impl<TOrd, P> EventHandler<LedgerTxEvent> for PendingOrderHandler<TOrd, P>
where
    P: TryFromBox<TOrd>,
{
    fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx(tx) => {
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
                    None
                } else {
                    Some(LedgerTxEvent::AppliedTx(tx))
                }
            }
            ev => Some(ev),
        }
    }
}
