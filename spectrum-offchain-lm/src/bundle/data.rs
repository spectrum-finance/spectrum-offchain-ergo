use ergo_lib::ergo_chain_types::Digest32;
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::ergo_box::{
    BoxTokens, ErgoBox, ErgoBoxCandidate, NonMandatoryRegisterId, NonMandatoryRegisters, RegisterValue,
};
use ergo_lib::ergotree_ir::chain::token::{Token, TokenId};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::{Constant, TryExtractInto};
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};
use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox};

use crate::data::assets::{BundleKey, Tmp, VirtLq};
use crate::data::{BundleId, BundleStateId, PoolId};
use crate::ergo::NanoErg;
use crate::validators::bundle_validator;

/// Prototype of StakeingBundle which guards virtual liquidity and temporal tokens.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StakingBundleProto {
    pub bundle_key_id: TypedAsset<BundleKey>,
    pub pool_id: PoolId,
    pub vlq: TypedAssetAmount<VirtLq>,
    pub tmp: TypedAssetAmount<Tmp>,
    pub redeemer_prop: ErgoTree,
    pub erg_value: NanoErg,
}

impl StakingBundleProto {
    pub fn finalize(self, state_id: BundleStateId) -> StakingBundle {
        StakingBundle {
            bundle_key_id: self.bundle_key_id,
            state_id,
            pool_id: self.pool_id,
            vlq: self.vlq,
            tmp: self.tmp,
            redeemer_prop: self.redeemer_prop,
            erg_value: self.erg_value,
        }
    }
}

impl IntoBoxCandidate for StakingBundleProto {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        let tokens = BoxTokens::from_vec(vec![Token::from(self.vlq), Token::from(self.tmp)]).unwrap();
        let registers = NonMandatoryRegisters::try_from(vec![
            RegisterValue::Parsed(Constant::from(
                self.redeemer_prop.sigma_serialize_bytes().unwrap(),
            )),
            RegisterValue::Parsed(Constant::from(
                self.bundle_key_id.token_id.sigma_serialize_bytes().unwrap(),
            )),
            RegisterValue::Parsed(Constant::from(
                TokenId::from(self.pool_id).sigma_serialize_bytes().unwrap(),
            )),
        ])
            .unwrap();
        ErgoBoxCandidate {
            value: BoxValue::from(self.erg_value),
            ergo_tree: bundle_validator(),
            tokens: Some(tokens),
            additional_registers: registers,
            creation_height: height,
        }
    }
}

/// Guards virtual liquidity and temporal tokens.
/// Staking Bundle is a persistent, self-reproducible, on-chain entity.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StakingBundle {
    pub bundle_key_id: TypedAsset<BundleKey>,
    pub state_id: BundleStateId,
    pub pool_id: PoolId,
    pub vlq: TypedAssetAmount<VirtLq>,
    pub tmp: TypedAssetAmount<Tmp>,
    pub redeemer_prop: ErgoTree,
    pub erg_value: NanoErg,
}

impl StakingBundle {
    pub fn from_proto(p: StakingBundleProto, state_id: BundleStateId) -> Self {
        Self {
            bundle_key_id: p.bundle_key_id,
            state_id,
            pool_id: p.pool_id,
            vlq: p.vlq,
            tmp: p.tmp,
            redeemer_prop: p.redeemer_prop,
            erg_value: p.erg_value,
        }
    }

    pub fn bundle_id(&self) -> BundleId {
        BundleId::from(self.bundle_key_id.token_id)
    }
}

impl From<StakingBundle> for StakingBundleProto {
    fn from(sb: StakingBundle) -> Self {
        Self {
            bundle_key_id: sb.bundle_key_id,
            pool_id: sb.pool_id,
            vlq: sb.vlq,
            tmp: sb.tmp,
            redeemer_prop: sb.redeemer_prop,
            erg_value: sb.erg_value,
        }
    }
}

impl OnChainEntity for StakingBundle {
    type TEntityId = BundleId;
    type TStateId = BundleStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.bundle_id()
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.state_id
    }
}

impl IntoBoxCandidate for StakingBundle {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        StakingBundleProto::from(self).into_candidate(height)
    }
}

impl TryFromBox for StakingBundle {
    fn try_from_box(bx: ErgoBox) -> Option<StakingBundle> {
        if let Some(ref tokens) = bx.tokens {
            if tokens.len() == 2 && bx.ergo_tree == bundle_validator() {
                let redeemer_prop = ErgoTree::sigma_parse_bytes(
                    &*bx.additional_registers
                        .get(NonMandatoryRegisterId::R4)?
                        .as_option_constant()?
                        .v
                        .clone()
                        .try_extract_into::<Vec<u8>>()
                        .ok()?,
                )
                    .ok()?;
                let bundle_key = TokenId::from(
                    Digest32::try_from(
                        bx.additional_registers
                            .get(NonMandatoryRegisterId::R5)?
                            .as_option_constant()?
                            .v
                            .clone()
                            .try_extract_into::<Vec<u8>>()
                            .ok()?,
                    )
                        .ok()?,
                );
                let pool_id = TokenId::from(
                    Digest32::try_from(
                        bx.additional_registers
                            .get(NonMandatoryRegisterId::R6)?
                            .as_option_constant()?
                            .v
                            .clone()
                            .try_extract_into::<Vec<u8>>()
                            .ok()?,
                    )
                        .ok()?,
                );
                let vlq = tokens.get(0)?.clone();
                let tmp = tokens.get(1)?.clone();
                return Some(StakingBundle {
                    bundle_key_id: TypedAsset::<BundleKey>::new(bundle_key),
                    state_id: BundleStateId::from(bx.box_id()),
                    pool_id: PoolId::from(pool_id),
                    vlq: TypedAssetAmount::<VirtLq>::from_token(vlq),
                    tmp: TypedAssetAmount::<Tmp>::from_token(tmp),
                    redeemer_prop,
                    erg_value: NanoErg::from(bx.value),
                });
            }
        }
        None
    }
}
