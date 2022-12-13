use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;
use spectrum_offchain::data::unique_entity::{Confirmed, Upgrade, UpgradeRollback};
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;
use crate::data::bundle::StakingBundle;

pub struct ConfirmedBundleUpgradeHadler {
    topic: UnboundedSender<Upgrade<Confirmed<StakingBundle>>>,
}

#[async_trait(?Send)]
impl EventHandler<LedgerTxEvent> for ConfirmedBundleUpgradeHadler {
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        todo!()
    }
}

pub struct ConfirmedBundleRollbackHadler {
    topic: UnboundedSender<UpgradeRollback<Confirmed<StakingBundle>>>,
}

#[async_trait(?Send)]
impl EventHandler<LedgerTxEvent> for ConfirmedBundleRollbackHadler {
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        todo!()
    }
}
