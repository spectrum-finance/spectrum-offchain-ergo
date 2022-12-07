use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBoxCandidate;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::domain::TypedAssetAmount;
use spectrum_offchain::event_sink::handlers::types::IntoBoxCandidate;

use crate::data::assets::{BundleKey, Lq, Reward};

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct RewardOutput {
    pub reward: TypedAssetAmount<Reward>,
    pub redeemer_prop: ErgoTree,
}

impl IntoBoxCandidate for RewardOutput {
    fn into_candidate(self) -> ErgoBoxCandidate {
        todo!()
    }
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct DepositOutput {
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub redeemer_prop: ErgoTree,
}

impl IntoBoxCandidate for DepositOutput {
    fn into_candidate(self) -> ErgoBoxCandidate {
        todo!()
    }
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct RedeemOutput {
    pub lq: TypedAssetAmount<Lq>,
    pub redeemer_prop: ErgoTree,
}

impl IntoBoxCandidate for RedeemOutput {
    fn into_candidate(self) -> ErgoBoxCandidate {
        todo!()
    }
}
