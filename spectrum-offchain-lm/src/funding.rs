use async_trait::async_trait;

use spectrum_offchain::data::unique_entity::{Confirmed, Predicted};

use crate::data::{AsBox, FundingId};
use crate::ergo::NanoErg;
use crate::funding::data::DistributionFunding;

pub mod data;
pub mod process;

#[async_trait]
pub trait FundingRepo {
    async fn select(&mut self, amount: NanoErg) -> Vec<AsBox<DistributionFunding>>;
    async fn put_confirmed(&mut self, df: Confirmed<AsBox<DistributionFunding>>);
    async fn put_predicted(&mut self, df: Predicted<AsBox<DistributionFunding>>);
    async fn exists(&self, fid: FundingId) -> bool;
    async fn remove(&mut self, fid: FundingId);
}
