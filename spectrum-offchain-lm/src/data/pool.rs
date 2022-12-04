use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::domain::TypedAssetAmount;

use crate::data::assets::{Lq, Rew, Tmp, VirtLq};
use crate::data::{PoolId, PoolStateId};

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct ProgramConfig {
    epoch_len: u32,
    epoch_size: u32,
    program_start: u32,
    program_budget: u64,
    min_value: u64,
    exec_budget: Option<u64>,
    epoch: Option<u32>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Pool {
    pool_id: PoolId,
    state_id: PoolStateId,
    budget_rew: TypedAssetAmount<Rew>,
    reserves_lq: TypedAssetAmount<Lq>,
    reserves_vlq: TypedAssetAmount<VirtLq>,
    reserves_tmp: TypedAssetAmount<Tmp>,
    conf: ProgramConfig,
}

impl OnChainEntity for Pool {
    type TEntityId = PoolId;
    type TStateId = PoolStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.pool_id.clone() // todo: remove .clone() when sigma is updated.
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.state_id.clone() // todo: remove .clone() when sigma is updated.
    }
}
