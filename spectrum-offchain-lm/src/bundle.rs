use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use spectrum_offchain::data::unique_entity::{Confirmed, Predicted, Traced, Unconfirmed};

use crate::data::bundle::StakingBundle;
use crate::data::{AsBox, BundleId, BundleStateId, PoolId};

pub mod process;

#[async_trait]
pub trait BundleRepo {
    /// Select bundles corresponding to
    async fn select(&self, pool_id: PoolId, epoch_ix: u32) -> Vec<BundleId>;
    /// Get particular state of staking bundle.
    async fn get_state(&self, state_id: BundleStateId) -> Option<StakingBundle>;
    /// Invalidate bundle state snapshot corresponding to the given `state_id`.
    async fn invalidate(&self, state_id: BundleStateId);

    async fn put_confirmed(&self, bundle: Confirmed<AsBox<StakingBundle>>);

    async fn put_unconfirmed(&self, bundle: Unconfirmed<AsBox<StakingBundle>>);

    async fn put_predicted(&self, bundle: Traced<Predicted<AsBox<StakingBundle>>>);
}

pub async fn resolve_bundle_state<TRepo>(
    bundle_id: BundleId,
    repo: Arc<Mutex<TRepo>>,
) -> Option<AsBox<StakingBundle>>
where
    TRepo: BundleRepo,
{
    todo!()
}
