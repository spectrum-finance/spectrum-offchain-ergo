use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};
use spectrum_offchain::event_sink::handlers::types::TryFromBox;

use crate::data::assets::{BundleKey, PoolNft, Tmp, VirtLq};
use crate::data::{BundleId, BundleStateId};

/// Guards virtual liquidity and temporal tokens.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StakingBundle {
    bundle_key: TypedAsset<BundleKey>,
    state_id: BundleStateId,
    pool_id: TypedAsset<PoolNft>,
    vlq: TypedAssetAmount<VirtLq>,
    tmp: TypedAssetAmount<Tmp>,
    redeemer: ErgoTree,
}

impl StakingBundle {
    pub fn bundle_id(&self) -> BundleId {
        BundleId::from(self.bundle_key.clone()) // todo: remove .clone() when sigma is updated.
    }
}

impl OnChainEntity for StakingBundle {
    type TEntityId = BundleId;
    type TStateId = BundleStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.bundle_id()
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.state_id.clone() // todo: remove .clone() when sigma is updated.
    }
}

pub struct BundleParser {
    bundle_validator_template: Vec<u8>,
}

impl TryFromBox<StakingBundle> for BundleParser {
    fn try_from(&self, bx: ErgoBox) -> Option<StakingBundle> {
        todo!()
    }
}
