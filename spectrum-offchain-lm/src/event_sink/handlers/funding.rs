use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;

use spectrum_offchain::data::unique_entity::Confirmed;
use spectrum_offchain::event_sink::handlers::types::TryFromBoxCtx;
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;

use crate::data::{AsBox, FundingId};
use crate::funding::data::{DistributionFunding, EliminatedFunding, ExecutorWallet};
use crate::funding::FundingRepo;

pub struct ConfirmedFundingHadler {
    topic: UnboundedSender<Confirmed<AsBox<DistributionFunding>>>,
    wallet: ExecutorWallet,
}

#[async_trait(?Send)]
impl EventHandler<LedgerTxEvent> for ConfirmedFundingHadler {
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for bx in &tx.outputs {
                    if let Some(order) = DistributionFunding::try_from_box(bx.clone(), self.wallet.clone()) {
                        is_success = true;
                        let _ = self.topic.send(Confirmed(AsBox(bx.clone(), order)));
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

pub struct ConfirmedFundingDiscardHadler<TFundingRepo> {
    topic: UnboundedSender<EliminatedFunding>,
    funding_repo: Arc<Mutex<TFundingRepo>>,
    wallet: ExecutorWallet,
}

#[async_trait(?Send)]
impl<TFundingRepo> EventHandler<LedgerTxEvent> for ConfirmedFundingDiscardHadler<TFundingRepo>
where
    TFundingRepo: FundingRepo,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                let funding_repo = self.funding_repo.lock();
                for i in tx.clone().inputs {
                    let fid = FundingId::from(i.box_id);
                    if funding_repo.exists(fid.clone()).await {
                        is_success = true;
                        let _ = self.topic.send(EliminatedFunding(fid));
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
                    if let Some(funding) = DistributionFunding::try_from_box(bx.clone(), self.wallet.clone())
                    {
                        is_success = true;
                        let _ = self.topic.send(EliminatedFunding(funding.id));
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
