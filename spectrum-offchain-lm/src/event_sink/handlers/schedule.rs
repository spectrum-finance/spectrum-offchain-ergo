use std::sync::Arc;

use async_trait::async_trait;
use log::trace;
use tokio::sync::Mutex;

use spectrum_offchain::box_resolver::persistence::EntityRepo;
use spectrum_offchain::event_sink::handlers::types::TryFromBox;
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;

use crate::data::pool::Pool;
use crate::data::{AsBox, PoolStateId};
use crate::scheduler::data::PoolSchedule;
use crate::scheduler::ScheduleRepo;

pub struct ConfirmedScheduleUpdateHandler<TRepo, TPools> {
    pub schedules: Arc<Mutex<TRepo>>,
    pub pools: Arc<Mutex<TPools>>,
}

impl<TRepo, TPools> ConfirmedScheduleUpdateHandler<TRepo, TPools> {
    pub fn new(schedules: Arc<Mutex<TRepo>>, pools: Arc<Mutex<TPools>>) -> Self {
        Self { schedules, pools }
    }
}

#[async_trait(?Send)]
impl<TRepo, TPools> EventHandler<LedgerTxEvent> for ConfirmedScheduleUpdateHandler<TRepo, TPools>
where
    TRepo: ScheduleRepo,
    TPools: EntityRepo<AsBox<Pool>>,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for o in &tx.outputs {
                    if let Some(pool) = Pool::try_from_box(o.clone()) {
                        let mut repo = self.schedules.lock().await;
                        let pid = pool.pool_id;

                        trace!(
                            target: "offchain_lm",
                            "LedgerTxEvent::AppliedTx: pool_box_id: {:?}, epoch_ix: {:?}, budget_rem: {:?}",
                            o.box_id(),
                            pool.epoch_ix,
                            pool.budget_rem
                        );

                        if let Err(_exhausted) = repo.update_schedule(PoolSchedule::from(pool)).await {
                            repo.clean(pid).await;
                        }
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
            LedgerTxEvent::UnappliedTx(tx) => {
                let mut is_success = false;
                let pools = self.pools.lock().await;
                for i in &tx.inputs {
                    let sid = PoolStateId::from(i.box_id);
                    if pools.may_exist(sid).await {
                        if let Some(AsBox(_, pool)) = pools.get_state(sid).await {
                            let mut repo = self.schedules.lock().await;
                            let pid = pool.pool_id;
                            if let Err(_exhausted) = repo.update_schedule(PoolSchedule::from(pool)).await {
                                repo.clean(pid).await;
                            }
                            is_success = true;
                        }
                    }
                }
                if is_success {
                    trace!(target: "offchain_lm", "Revived schedule parsed from unapplied tx");
                    None
                } else {
                    Some(LedgerTxEvent::UnappliedTx(tx))
                }
            }
        }
    }
}
