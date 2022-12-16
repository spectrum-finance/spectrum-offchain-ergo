use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;

use spectrum_offchain::data::unique_entity::{Confirmed, Upgrade, UpgradeRollback};
use spectrum_offchain::event_sink::handlers::types::TryFromBox;
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;

use crate::data::bundle::StakingBundle;
use crate::bundle::BundleRepo;
use crate::data::{AsBox, BundleStateId};

pub struct ConfirmedBundleUpgradeHadler {
    topic: UnboundedSender<Upgrade<Confirmed<AsBox<StakingBundle>>>>,
}

#[async_trait(?Send)]
impl EventHandler<LedgerTxEvent> for ConfirmedBundleUpgradeHadler {
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for bx in &tx.outputs {
                    if let Some(order) = StakingBundle::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self.topic.send(Upgrade(Confirmed(AsBox(bx.clone(), order))));
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

pub struct ConfirmedBundleRollbackHadler<TBundleRepo> {
    topic: UnboundedSender<UpgradeRollback<StakingBundle>>,
    bundle_repo: Arc<Mutex<TBundleRepo>>,
}

#[async_trait(?Send)]
impl<TBundleRepo> EventHandler<LedgerTxEvent> for ConfirmedBundleRollbackHadler<TBundleRepo>
where
    TBundleRepo: BundleRepo,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for i in tx.clone().inputs {
                    let state_id = BundleStateId::from(i.box_id);
                    if let Some(entity_snapshot) = self.bundle_repo.lock().get_state(state_id).await {
                        is_success = true;
                        let _ = self.topic.send(UpgradeRollback(entity_snapshot));
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
                    if let Some(entity) = StakingBundle::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self.topic.send(UpgradeRollback(entity));
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
