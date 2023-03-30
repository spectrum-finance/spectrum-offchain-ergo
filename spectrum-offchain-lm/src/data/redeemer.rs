use ergo_lib::ergotree_ir::chain::ergo_box::{
    BoxTokens, ErgoBoxCandidate, NonMandatoryRegisters, RegisterValue,
};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::Constant;
use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::SigmaProp;

use spectrum_offchain::domain::TypedAssetAmount;
use spectrum_offchain::event_sink::handlers::types::IntoBoxCandidate;

use crate::data::assets::{BundleKey, Lq, Reward};
use crate::ergo::{NanoErg, DEFAULT_P2PK_HEADER};

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct RewardOutput {
    pub reward: Option<TypedAssetAmount<Reward>>,
    pub redeemer_prop: SigmaProp,
    pub erg_value: NanoErg,
}

impl IntoBoxCandidate for RewardOutput {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        let tokens = self
            .reward
            .map(|reward| BoxTokens::from([reward.try_into().unwrap()]));
        ErgoBoxCandidate {
            value: self.erg_value.into(),
            ergo_tree: ErgoTree::new(DEFAULT_P2PK_HEADER.clone(), &self.redeemer_prop.into()).unwrap(),
            tokens,
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: height,
        }
    }
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct DepositOutput {
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub redeemer_prop: SigmaProp,
    pub erg_value: NanoErg,
    pub token_name: String,
    pub token_desc: String,
}

impl IntoBoxCandidate for DepositOutput {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        ErgoBoxCandidate {
            value: self.erg_value.into(),
            ergo_tree: ErgoTree::new(DEFAULT_P2PK_HEADER.clone(), &self.redeemer_prop.into()).unwrap(),
            tokens: Some(BoxTokens::from([self.bundle_key.try_into().unwrap()])),
            additional_registers: NonMandatoryRegisters::try_from(vec![
                RegisterValue::Parsed(Constant::from(self.token_name.as_bytes().to_vec())),
                RegisterValue::Parsed(Constant::from(self.token_desc.as_bytes().to_vec())),
            ])
            .unwrap(),
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
            tokens: Some(BoxTokens::from([self.lq.try_into().unwrap()])),
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: height,
        }
    }
}
