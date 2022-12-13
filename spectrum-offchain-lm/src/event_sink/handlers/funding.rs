use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

use spectrum_offchain::data::unique_entity::Confirmed;
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;

use crate::data::executor::DistributionFunding;
use crate::data::{AsBox, Discarded};
use crate::funding::FundingRepo;

pub struct ConfirmedFundingHadler<TFundingRepo> {
    topic: UnboundedSender<Confirmed<AsBox<DistributionFunding>>>,
    funding_repo: TFundingRepo,
}

#[async_trait(?Send)]
impl<TFundingRepo> EventHandler<LedgerTxEvent> for ConfirmedFundingHadler<TFundingRepo>
where
    TFundingRepo: FundingRepo,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        todo!()
    }
}

pub struct ConfirmedFundingDiscardHadler<TFundingRepo> {
    topic: UnboundedSender<Discarded<Confirmed<DistributionFunding>>>,
    funding_repo: TFundingRepo,
}

#[async_trait(?Send)]
impl<TFundingRepo> EventHandler<LedgerTxEvent> for ConfirmedFundingDiscardHadler<TFundingRepo>
where
    TFundingRepo: FundingRepo,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        todo!()
    }
}
