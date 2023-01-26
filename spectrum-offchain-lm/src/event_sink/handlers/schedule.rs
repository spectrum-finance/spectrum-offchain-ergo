use std::sync::Arc;

use async_trait::async_trait;
use log::trace;
use tokio::sync::Mutex;

use spectrum_offchain::event_sink::handlers::types::TryFromBox;
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;

use crate::data::pool::Pool;
use crate::scheduler::data::PoolSchedule;
use crate::scheduler::ScheduleRepo;

pub struct ConfirmedScheduleUpdateHandler<TRepo> {
    pub schedules: Arc<Mutex<TRepo>>,
}

impl<TRepo> ConfirmedScheduleUpdateHandler<TRepo> {
    pub fn new(schedules: Arc<Mutex<TRepo>>) -> Self {
        Self { schedules }
    }
}

#[async_trait(?Send)]
impl<TRepo> EventHandler<LedgerTxEvent> for ConfirmedScheduleUpdateHandler<TRepo>
where
    TRepo: ScheduleRepo,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for o in &tx.outputs {
                    if let Some(pool) = Pool::try_from_box(o.clone()) {
                        let mut repo = self.schedules.lock().await;
                        repo.put_schedule(PoolSchedule::from(pool)).await;
                        is_success = true;
                    }
                }
                if is_success {
                    trace!(target: "offchain_lm", "New schedule parsed from applied tx");
                    None
                } else {
                    Some(LedgerTxEvent::AppliedTx { tx, timestamp })
                }
            }
            ev => Some(ev),
        }
    }
}
