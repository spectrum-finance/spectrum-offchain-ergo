use ergo_lib::ergotree_ir::chain::ergo_box::{BoxTokens, ErgoBoxCandidate, NonMandatoryRegisters};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::domain::TypedAssetAmount;
use spectrum_offchain::event_sink::handlers::types::IntoBoxCandidate;

use crate::data::assets::{BundleKey, Lq, Reward};
use crate::ergo::NanoErg;

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct RewardOutput {
    pub reward: TypedAssetAmount<Reward>,
    pub redeemer_prop: ErgoTree,
    pub erg_value: NanoErg,
}

impl IntoBoxCandidate for RewardOutput {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        ErgoBoxCandidate {
            value: self.erg_value.into(),
            ergo_tree: self.redeemer_prop,
            tokens: Some(BoxTokens::from([self.reward.into()])),
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: height,
        }
    }
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct DepositOutput {
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub redeemer_prop: ErgoTree,
    pub erg_value: NanoErg,
}

impl IntoBoxCandidate for DepositOutput {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        ErgoBoxCandidate {
            value: self.erg_value.into(),
            ergo_tree: self.redeemer_prop,
            tokens: Some(BoxTokens::from([self.bundle_key.into()])),
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: height,
        }
    }
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct RedeemOutput {
    pub lq: TypedAssetAmount<Lq>,
    pub redeemer_prop: ErgoTree,
    pub erg_value: NanoErg,
}

impl IntoBoxCandidate for RedeemOutput {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        ErgoBoxCandidate {
            value: self.erg_value.into(),
            ergo_tree: self.redeemer_prop,
            tokens: Some(BoxTokens::from([self.lq.into()])),
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: height,
        }
    }
}
