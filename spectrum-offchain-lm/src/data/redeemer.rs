use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::domain::TypedAssetAmount;

use crate::data::assets::{BundleKey, Lq, Reward};

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct RewardOutput {
    pub reward: TypedAssetAmount<Reward>,
    pub redeemer_prop: ErgoTree,
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct DepositOutput {
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub redeemer_prop: ErgoTree,
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct RedeemOutput {
    pub lq: TypedAssetAmount<Lq>,
    pub redeemer_prop: ErgoTree,
}
