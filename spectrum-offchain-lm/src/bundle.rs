use async_trait::async_trait;

use spectrum_offchain::data::unique_entity::{Confirmed, Predicted, Traced};

use crate::bundle::data::StakingBundle;
use crate::data::{AsBox, BundleId, BundleStateId, PoolId};

pub mod data;
pub mod process;

#[async_trait]
pub trait BundleRepo {
    async fn put_confirmed(&self, bundle: Confirmed<AsBox<StakingBundle>>);
    async fn put_predicted(&self, bundle: Traced<Predicted<AsBox<StakingBundle>>>);
    /// Select bundles corresponding to 
    async fn select(&self, pool_id: PoolId, epoch_ix: u32) -> Vec<BundleId>;
    /// Get the latest state of bundle with the given `bundle_id`.
    async fn get(&self, bundle_id: BundleId) -> Option<AsBox<StakingBundle>>;
    /// Get particular state of staking bundle.
    async fn get_state(&self, state_id: BundleStateId) -> Option<StakingBundle>;
    /// Invalidate bundle state snapshot corresponding to the given `state_id`.
    async fn invalidate(&self, state_id: BundleStateId);
}
