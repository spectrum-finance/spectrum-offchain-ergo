use std::sync::Arc;

use async_trait::async_trait;
use log::trace;
use tokio::sync::Mutex;

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::event_sink::handlers::types::TryFromBox;
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;

use crate::data::pool::Pool;
use crate::program::ProgramRepo;

pub struct ConfirmedProgramUpdateHandler<TRepo> {
    pub programs: Arc<Mutex<TRepo>>,
}

impl<TRepo> ConfirmedProgramUpdateHandler<TRepo> {
    pub fn new(programs: Arc<Mutex<TRepo>>) -> Self {
        Self { programs }
    }
}

#[async_trait(?Send)]
impl<TRepo> EventHandler<LedgerTxEvent> for ConfirmedProgramUpdateHandler<TRepo>
where
    TRepo: ProgramRepo,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for o in &tx.outputs {
                    if let Some(pool) = Pool::try_from_box(o.clone()) {
                        let repo = self.programs.lock().await;
                        if !repo.exists(pool.pool_id).await {
                            is_success = true;
                            repo.put(pool.pool_id, pool.conf).await;
                        }
                    }
                }
                if is_success {
                    trace!(target: "offchain_lm", "New program parsed from applied tx");
                    None
                } else {
                    Some(LedgerTxEvent::AppliedTx { tx, timestamp })
                }
            }
            ev => Some(ev),
        }
    }
}
