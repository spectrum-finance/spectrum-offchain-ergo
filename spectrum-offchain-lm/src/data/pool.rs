use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, NonMandatoryRegisterId};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::TryExtractInto;

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};
use spectrum_offchain::event_sink::handlers::types::TryFromBox;

use crate::data::assets::{Budget, Lq, PoolNft, Tmp, VirtLq};
use crate::data::{PoolId, PoolStateId};

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct ProgramConfig {
    pub epoch_len: u32,
    pub epoch_num: u32,
    pub program_start: u32,
    pub program_budget: TypedAssetAmount<Budget>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Pool {
    pub pool_id: PoolId,
    pub state_id: PoolStateId,
    pub budget_rem: TypedAssetAmount<Budget>,
    pub reserves_lq: TypedAssetAmount<Lq>,
    pub reserves_vlq: TypedAssetAmount<VirtLq>,
    pub reserves_tmp: TypedAssetAmount<Tmp>,
    pub conf: ProgramConfig,
}

impl Pool {
    pub fn epoch_alloc(&self) -> u64 {
        self.conf.program_budget.amount / self.conf.epoch_num as u64
    }

    pub fn num_epochs_remain(&self, current_height: u32) -> u32 {
        self.conf.epoch_num - (current_height - self.conf.program_start) / self.conf.epoch_len
    }
}

impl OnChainEntity for Pool {
    type TEntityId = PoolId;
    type TStateId = PoolStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.pool_id
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.state_id
    }
}

pub struct PoolParser {
    pub pool_validator: ErgoTree
}

impl TryFromBox<Pool> for PoolParser {
    fn try_from(&self, bx: ErgoBox) -> Option<Pool> {
        let r4 = bx.additional_registers.get(NonMandatoryRegisterId::R4);
        let r5 = bx.additional_registers.get(NonMandatoryRegisterId::R5);
        if let (Some(r4), Some(r5), Some(tokens)) = (r4, r5, &bx.tokens) {
            if tokens.len() == 5 && self.pool_validator == bx.ergo_tree {
                let pool_nft = tokens.get(0).unwrap().token_id;
                let budget_rem = tokens.get(1)?;
                let lq = tokens.get(2)?;
                let vlq = tokens.get(3)?;
                let tmp = tokens.get(4)?;
                let conf = r4
                    .as_option_constant()
                    .map(|c| c.clone().try_extract_into::<Vec<i32>>())?
                    .ok()?;
                let budget = r5
                    .as_option_constant()
                    .map(|c| c.clone().try_extract_into::<i64>())?
                    .ok()?;
                let conf = ProgramConfig {
                    epoch_len: *conf.get(0)? as u32,
                    epoch_num: *conf.get(1)? as u32,
                    program_start: *conf.get(2)? as u32,
                    program_budget: TypedAssetAmount::new(budget_rem.token_id, budget as u64),
                };
                return Some(Pool {
                    pool_id: PoolId::from(TypedAsset::<PoolNft>::new(pool_nft)),
                    state_id: PoolStateId::from(bx.box_id()),
                    budget_rem: TypedAssetAmount::new(
                        budget_rem.token_id,
                        *budget_rem.amount.as_u64(),
                    ),
                    reserves_lq: TypedAssetAmount::new(lq.token_id, *lq.amount.as_u64()),
                    reserves_vlq: TypedAssetAmount::new(vlq.token_id, *vlq.amount.as_u64()),
                    reserves_tmp: TypedAssetAmount::new(tmp.token_id, *tmp.amount.as_u64()),
                    conf,
                });
            }
        }
        None
    }
}
