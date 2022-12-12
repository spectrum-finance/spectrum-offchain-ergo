use async_trait::async_trait;
use spectrum_offchain::data::unique_entity::{Predicted, Traced};
use crate::data::bundle::StakingBundle;
use crate::data::{AsBox, BundleId, BundleStateId, PoolId};

#[async_trait]
pub trait BundleRepo {
    async fn put_predicted(&self, bundle: Traced<Predicted<AsBox<StakingBundle>>>);
    async fn select(&self, pool_id: PoolId, epoch_ix: u32) -> Vec<BundleId>;
    async fn get(&self, bundle_id: BundleId) -> Option<AsBox<StakingBundle>>;
    async fn invalidate(&self, state_id: BundleStateId);
}