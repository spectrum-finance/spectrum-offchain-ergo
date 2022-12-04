use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};

use crate::data::assets::{BundleKey, PoolId, VirtLQ, TMP};

/// Guards virtual liquidity and temporal tokens.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StakingBundle {
    bundle_key: TypedAsset<BundleKey>,
    pool_id: TypedAsset<PoolId>,
    vlq: TypedAssetAmount<VirtLQ>,
    tmp: TypedAssetAmount<TMP>,
    redeemer: ErgoTree,
}
