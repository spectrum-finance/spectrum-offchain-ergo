use std::sync::Arc;

use async_trait::async_trait;
use futures::{Sink, SinkExt};
use parking_lot::Mutex;

use spectrum_offchain::data::unique_entity::Confirmed;
use spectrum_offchain::event_sink::handlers::types::TryFromBoxCtx;
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;

use crate::data::funding::{DistributionFunding, ExecutorWallet, FundingUpdate};
use crate::data::{AsBox, FundingId};
use crate::funding::FundingRepo;

pub struct ConfirmedFundingHadler<TSink, TRepo> {
    pub topic: TSink,
    pub repo: Arc<Mutex<TRepo>>,
    pub wallet: ExecutorWallet,
}

#[async_trait(?Send)]
impl<TSink, TRepo> EventHandler<LedgerTxEvent> for ConfirmedFundingHadler<TSink, TRepo>
where
    TSink: Sink<Confirmed<FundingUpdate>> + Unpin,
    TRepo: FundingRepo,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        let res = match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for i in tx.clone().inputs {
                    let fid = FundingId::from(i.box_id);
                    if self.repo.lock().may_exist(fid.clone()).await {
                        is_success = true;
                        let _ = self
                            .topic
                            .feed(Confirmed(FundingUpdate::FundingEliminated(fid)))
                            .await;
                    }
                }
                for bx in &tx.outputs {
                    if let Some(funding) = DistributionFunding::try_from_box(bx.clone(), self.wallet.clone())
                    {
                        is_success = true;
                        let _ = self
                            .topic
                            .feed(Confirmed(FundingUpdate::FundingCreated(AsBox(
                                bx.clone(),
                                funding,
                            ))))
                            .await;
                    }
                }
                if is_success {
                    None
                } else {
                    Some(LedgerTxEvent::AppliedTx { tx, timestamp })
                }
            }
            LedgerTxEvent::UnappliedTx(tx) => {
                let mut is_success = false;
                for bx in &tx.outputs {
                    if let Some(funding) = DistributionFunding::try_from_box(bx.clone(), self.wallet.clone())
                    {
                        is_success = true;
                        let _ = self
                            .topic
                            .feed(Confirmed(FundingUpdate::FundingEliminated(funding.id)))
                            .await;
                    }
                }
                if is_success {
                    None
                } else {
                    Some(LedgerTxEvent::UnappliedTx(tx))
                }
            }
        };
        let _ = self.topic.flush().await;
        res
    }
}
