use ergo_lib::ergo_chain_types::Digest32;
use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, NonMandatoryRegisterId};
use ergo_lib::ergotree_ir::chain::token::TokenId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::TryExtractInto;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};
use spectrum_offchain::event_sink::handlers::types::TryFromBox;

use crate::data::assets::{BundleKey, Tmp, VirtLq};
use crate::data::{BundleId, BundleStateId, PoolId};
use crate::validators::bundle_validator;

/// Prototype of StakeingBundle which guards virtual liquidity and temporal tokens.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StakingBundleProto {
    pub bundle_key_id: TypedAsset<BundleKey>,
    pub pool_id: PoolId,
    pub vlq: TypedAssetAmount<VirtLq>,
    pub tmp: TypedAssetAmount<Tmp>,
    pub redeemer_prop: ErgoTree,
}

/// Guards virtual liquidity and temporal tokens.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StakingBundle {
    pub bundle_key_id: TypedAsset<BundleKey>,
    pub state_id: BundleStateId,
    pub pool_id: PoolId,
    pub vlq: TypedAssetAmount<VirtLq>,
    pub tmp: TypedAssetAmount<Tmp>,
    pub redeemer_prop: ErgoTree,
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
        }
    }
}

impl StakingBundle {
    pub fn bundle_id(&self) -> BundleId {
        BundleId::from(self.bundle_key_id.token_id)
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
                });
            }
        }
        None
    }
}
