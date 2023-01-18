use ergo_lib::ergo_chain_types::Digest32;
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::ergo_box::{
    BoxTokens, ErgoBox, ErgoBoxCandidate, NonMandatoryRegisterId, NonMandatoryRegisters, RegisterValue,
};
use ergo_lib::ergotree_ir::chain::token::{Token, TokenAmount, TokenId};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::{Constant, TryExtractInto};
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use serde::{Deserialize, Serialize};

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};
use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox};

use crate::data::assets::{BundleKey, Tmp, VirtLq};
use crate::data::pool::{ProgramConfig, INIT_EPOCH_IX};
use crate::data::{BundleId, BundleStateId, PoolId};
use crate::ergo::{NanoErg, MAX_VALUE};
use crate::validators::BUNDLE_VALIDATOR;

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
        let tokens = BoxTokens::from_vec(vec![
            Token::from(self.vlq),
            Token::from(self.tmp),
            Token {
                token_id: self.bundle_key_id.token_id,
                amount: TokenAmount::try_from(BUNDLE_KEY_AMOUNT).unwrap(),
            },
        ])
        .unwrap();
        let registers = NonMandatoryRegisters::try_from(vec![
            RegisterValue::Parsed(Constant::from(
                self.redeemer_prop.sigma_serialize_bytes().unwrap(),
            )),
            RegisterValue::Parsed(Constant::from(
                TokenId::from(self.pool_id).sigma_serialize_bytes().unwrap(),
            )),
        ])
        .unwrap();
        ErgoBoxCandidate {
            value: BoxValue::from(self.erg_value),
            ergo_tree: BUNDLE_VALIDATOR.clone(),
            tokens: Some(tokens),
            additional_registers: registers,
            creation_height: height,
        }
    }
}

pub const BUNDLE_KEY_AMOUNT: u64 = 1;
pub const BUNDLE_KEY_AMOUNT_USER: u64 = MAX_VALUE - BUNDLE_KEY_AMOUNT;

/// Guards virtual liquidity and temporal tokens.
/// Staking Bundle is a persistent, self-reproducible, on-chain entity.
#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
#[serde(from = "StakingBundleWithErgoTreeBytes")]
#[serde(into = "StakingBundleWithErgoTreeBytes")]
pub struct StakingBundle {
    pub bundle_key_id: TypedAsset<BundleKey>,
    pub state_id: BundleStateId,
    pub pool_id: PoolId,
    pub vlq: TypedAssetAmount<VirtLq>,
    pub tmp: TypedAssetAmount<Tmp>,
    pub redeemer_prop: ErgoTree,
    pub erg_value: NanoErg,
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
struct StakingBundleWithErgoTreeBytes {
    bundle_key_id: TypedAsset<BundleKey>,
    state_id: BundleStateId,
    pool_id: PoolId,
    vlq: TypedAssetAmount<VirtLq>,
    tmp: TypedAssetAmount<Tmp>,
    /// Sigma-serialized byte representation of `ErgoTree`
    redeemer_prop_bytes: Vec<u8>,
    erg_value: NanoErg,
}

impl From<StakingBundleWithErgoTreeBytes> for StakingBundle {
    fn from(s: StakingBundleWithErgoTreeBytes) -> Self {
        Self {
            bundle_key_id: s.bundle_key_id,
            state_id: s.state_id,
            pool_id: s.pool_id,
            vlq: s.vlq,
            tmp: s.tmp,
            redeemer_prop: ErgoTree::sigma_parse_bytes(&s.redeemer_prop_bytes).unwrap(),
            erg_value: s.erg_value,
        }
    }
}

impl From<StakingBundle> for StakingBundleWithErgoTreeBytes {
    fn from(s: StakingBundle) -> Self {
        Self {
            bundle_key_id: s.bundle_key_id,
            state_id: s.state_id,
            pool_id: s.pool_id,
            vlq: s.vlq,
            tmp: s.tmp,
            redeemer_prop_bytes: s.redeemer_prop.sigma_serialize_bytes().unwrap(),
            erg_value: s.erg_value,
        }
    }
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
            if tokens.len() == 3 && bx.ergo_tree == *BUNDLE_VALIDATOR {
                let redeemer_prop = ErgoTree::sigma_parse_bytes(
                    &bx.get_register(NonMandatoryRegisterId::R4.into())?
                        .v
                        .try_extract_into::<Vec<u8>>()
                        .ok()?,
                )
                .ok()?;
                let pool_id = TokenId::from(
                    Digest32::try_from(
                        bx.get_register(NonMandatoryRegisterId::R5.into())?
                            .v
                            .try_extract_into::<Vec<u8>>()
                            .ok()?,
                    )
                    .ok()?,
                );
                let vlq = tokens.get(0)?.clone();
                let tmp = tokens.get(1)?.clone();
                let bundle_key = tokens.get(2)?.clone();
                return Some(StakingBundle {
                    bundle_key_id: TypedAsset::new(bundle_key.token_id),
                    state_id: BundleStateId::from(bx.box_id()),
                    pool_id: PoolId::from(pool_id),
                    vlq: TypedAssetAmount::from_token(vlq),
                    tmp: TypedAssetAmount::from_token(tmp),
                    redeemer_prop,
                    erg_value: NanoErg::from(bx.value),
                });
            }
        }
        None
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct IndexedBundle<B> {
    pub bundle: B,
    pub lower_epoch_ix: u32,
}

impl IndexedBundle<StakingBundle> {
    pub fn new(bundle: StakingBundle, conf: ProgramConfig) -> Self {
        Self {
            lower_epoch_ix: conf.epoch_num - (bundle.tmp.amount / bundle.vlq.amount) as u32 + 1,
            bundle,
        }
    }
    pub fn init(bundle: StakingBundle) -> Self {
        Self {
            bundle,
            lower_epoch_ix: INIT_EPOCH_IX,
        }
    }
}

pub type IndexedStakingBundle = IndexedBundle<StakingBundle>;

impl<T> OnChainEntity for IndexedBundle<T>
where
    T: OnChainEntity,
{
    type TEntityId = T::TEntityId;
    type TStateId = T::TStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.bundle.get_self_ref()
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.bundle.get_self_state_ref()
    }
}
