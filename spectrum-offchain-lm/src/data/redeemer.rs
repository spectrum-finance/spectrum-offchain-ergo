use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::domain::TypedAssetAmount;

use crate::data::assets::{BundleKey, Lq, Reward};

pub struct RewardOutput {
    reward: TypedAssetAmount<Reward>,
    redeemer_prop: ErgoTree,
}

pub struct DepositOutput {
    bundle_key: TypedAssetAmount<BundleKey>,
    redeemer_prop: ErgoTree,
}

pub struct RedeemOutput {
    lq: TypedAssetAmount<Lq>,
    redeemer_prop: ErgoTree,
}
