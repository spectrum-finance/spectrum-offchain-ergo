use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, NonMandatoryRegisterId};
use ergo_lib::ergotree_ir::mir::constant::TryExtractInto;

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::domain::TypedAssetAmount;
use spectrum_offchain::event_sink::handlers::types::TryFromBox;

use crate::data::assets::{Lq, Reward, Tmp, VirtLq};
use crate::data::bundle::{StakingBundle, StakingBundleProto};
use crate::data::context::ExecutionContext;
use crate::data::order::{Deposit, Redeem};
use crate::data::redeemer::{DepositOutput, RedeemOutput, RewardOutput};
use crate::data::{PoolId, PoolStateId};
use crate::ergo::MAX_VALUE;
use crate::validators::pool_validator;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct ProgramConfig {
    pub epoch_len: u32,
    pub epoch_num: u32,
    pub program_start: u32,
    pub program_budget: TypedAssetAmount<Reward>,
}

pub enum PoolOperationError {
    Permanent(PermanentError),
    Temporal(TemporalError),
}

pub enum PermanentError {
    LiqudityMismatch,
    ProgramExhausted,
}

pub enum TemporalError {
    LiquidityMoveBlocked,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Pool {
    pub pool_id: PoolId,
    pub state_id: PoolStateId,
    pub budget_rem: TypedAssetAmount<Reward>,
    pub reserves_lq: TypedAssetAmount<Lq>,
    pub reserves_vlq: TypedAssetAmount<VirtLq>,
    pub reserves_tmp: TypedAssetAmount<Tmp>,
    pub epoch_ix: Option<u32>,
    pub max_error: u64,
    pub conf: ProgramConfig,
}

impl Pool {
    pub fn apply_deposit(
        self,
        deposit: Deposit,
        ctx: ExecutionContext,
    ) -> Result<(Pool, StakingBundleProto, DepositOutput), PoolOperationError> {
        if self.num_epochs_remain(ctx.height) < 1 {
            return Err(PoolOperationError::Permanent(PermanentError::ProgramExhausted));
        }
        if !self.epoch_completed() {
            return Err(PoolOperationError::Temporal(TemporalError::LiquidityMoveBlocked));
        }
        let release_vlq = deposit.lq.coerce::<VirtLq>();
        let release_tmp_amt = self.num_epochs_remain(ctx.height) as u64 * release_vlq.amount;
        let release_tmp = TypedAssetAmount::new(self.reserves_tmp.token_id, release_tmp_amt);
        let mut next_pool = self;
        next_pool.reserves_lq = next_pool.reserves_lq + deposit.lq;
        next_pool.reserves_vlq = next_pool.reserves_vlq - release_vlq;
        next_pool.reserves_tmp = next_pool.reserves_tmp - release_tmp;
        let bundle_key = TypedAssetAmount::new(ctx.mintable_token_id, MAX_VALUE);
        let bundle = StakingBundleProto {
            bundle_key_id: bundle_key.to_asset(),
            pool_id: next_pool.pool_id,
            vlq: release_vlq,
            tmp: release_tmp,
            redeemer_prop: deposit.redeemer_prop.clone(),
        };
        let user_output = DepositOutput {
            bundle_key,
            redeemer_prop: deposit.redeemer_prop,
        };
        Ok((next_pool, bundle, user_output))
    }

    pub fn apply_redeem(
        self,
        redeem: Redeem,
        bundle: StakingBundle,
    ) -> Result<(Pool, RedeemOutput), PoolOperationError> {
        if redeem.expected_lq.amount != bundle.vlq.amount {
            return Err(PoolOperationError::Permanent(PermanentError::LiqudityMismatch));
        }
        if !self.epoch_completed() {
            return Err(PoolOperationError::Temporal(TemporalError::LiquidityMoveBlocked));
        }
        let release_lq = redeem.expected_lq;
        let mut next_pool = self;
        next_pool.reserves_lq = next_pool.reserves_lq - release_lq;
        next_pool.reserves_vlq = next_pool.reserves_vlq + bundle.vlq;
        next_pool.reserves_tmp = next_pool.reserves_tmp + bundle.tmp;
        let user_output = RedeemOutput {
            lq: release_lq,
            redeemer_prop: redeem.redeemer_prop,
        };
        Ok((next_pool, user_output))
    }

    pub fn distribute_rewards(
        self,
        bundles: Vec<StakingBundle>,
    ) -> (Pool, Vec<StakingBundle>, Vec<RewardOutput>) {
        let epoch_alloc = self.epoch_alloc();
        let epoch_to_process = if self.epoch_completed() {
            self.conf.epoch_num as u64 - (self.budget_rem.amount / epoch_alloc)
        } else {
            self.conf.epoch_num as u64 - (self.budget_rem.amount / epoch_alloc) - 1
        };
        let mut next_pool = self;
        let mut next_bundles = bundles;
        let mut reward_outputs = Vec::new();
        for mut bundle in &mut next_bundles {
            let reward_amt = next_pool.epoch_alloc() * bundle.vlq.amount / next_pool.reserves_lq.amount;
            let reward = TypedAssetAmount::new(next_pool.budget_rem.token_id, reward_amt);
            let reward_output = RewardOutput {
                reward,
                redeemer_prop: bundle.redeemer_prop.clone(),
            };
            let charged_tmp = bundle.vlq.coerce();
            reward_outputs.push(reward_output);
            bundle.tmp = bundle.tmp - charged_tmp;
            next_pool.reserves_tmp = next_pool.reserves_tmp + charged_tmp;
            next_pool.budget_rem = next_pool.budget_rem - reward;
        }
        next_pool.epoch_ix = Some(epoch_to_process as u32);
        (next_pool, next_bundles, reward_outputs)
    }

    fn epoch_alloc(&self) -> u64 {
        self.conf.program_budget.amount / self.conf.epoch_num as u64
    }

    fn num_epochs_remain(&self, height: u32) -> u32 {
        self.conf.epoch_num - self.current_epoch(height)
    }

    fn current_epoch(&self, height: u32) -> u32 {
        let cur_block_ix = height - self.conf.program_start + 1;
        let cur_epoch_ix_rem = cur_block_ix % self.conf.epoch_len;
        let cur_epoch_ix_r = cur_block_ix / self.conf.epoch_len;
        if cur_epoch_ix_rem > 0 {
            cur_epoch_ix_r + 1
        } else {
            cur_epoch_ix_r
        }
    }

    fn epoch_completed(&self) -> bool {
        self.budget_rem.amount as f64 % self.epoch_alloc() as f64 <= self.max_error as f64
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

impl TryFromBox for Pool {
    fn try_from_box(bx: ErgoBox) -> Option<Pool> {
        let r4 = bx.additional_registers.get(NonMandatoryRegisterId::R4);
        let r5 = bx.additional_registers.get(NonMandatoryRegisterId::R5);
        let r6 = bx.additional_registers.get(NonMandatoryRegisterId::R6);
        let r7 = bx.additional_registers.get(NonMandatoryRegisterId::R7);
        if let Some(tokens) = &bx.tokens {
            if tokens.len() == 5 && pool_validator() == bx.ergo_tree {
                let pool_nft = tokens.get(0).unwrap().token_id;
                let budget_rem = tokens.get(1)?;
                let lq = tokens.get(2)?;
                let vlq = tokens.get(3)?;
                let tmp = tokens.get(4)?;
                let conf = r4?
                    .as_option_constant()
                    .map(|c| c.clone().try_extract_into::<Vec<i32>>())?
                    .ok()?;
                let budget = r5?
                    .as_option_constant()
                    .map(|c| c.clone().try_extract_into::<i64>())?
                    .ok()?;
                let max_error = <u64>::try_from(
                    r6?.as_option_constant()
                        .map(|c| c.clone().try_extract_into::<i64>())?
                        .ok()?,
                )
                .ok()?;
                let epoch_ix = r7
                    .and_then(|reg| reg.as_option_constant())
                    .and_then(|c| c.clone().try_extract_into::<i32>().ok())
                    .and_then(|x| <u32>::try_from(x).ok());
                let conf = ProgramConfig {
                    epoch_len: *conf.get(0)? as u32,
                    epoch_num: *conf.get(1)? as u32,
                    program_start: *conf.get(2)? as u32,
                    program_budget: TypedAssetAmount::new(budget_rem.token_id, budget as u64),
                };
                return Some(Pool {
                    pool_id: PoolId::from(pool_nft),
                    state_id: PoolStateId::from(bx.box_id()),
                    budget_rem: TypedAssetAmount::new(budget_rem.token_id, *budget_rem.amount.as_u64()),
                    reserves_lq: TypedAssetAmount::new(lq.token_id, *lq.amount.as_u64()),
                    reserves_vlq: TypedAssetAmount::new(vlq.token_id, *vlq.amount.as_u64()),
                    reserves_tmp: TypedAssetAmount::new(tmp.token_id, *tmp.amount.as_u64()),
                    epoch_ix,
                    max_error,
                    conf,
                });
            }
        }
        None
    }
}
