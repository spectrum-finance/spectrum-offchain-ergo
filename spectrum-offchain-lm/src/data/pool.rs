use derive_more::Display;
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::ergo_box::{
    BoxTokens, ErgoBox, ErgoBoxCandidate, NonMandatoryRegisterId, NonMandatoryRegisters,
};
use ergo_lib::ergotree_ir::chain::token::Token;
use ergo_lib::ergotree_ir::mir::constant::{Constant, TryExtractInto};
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::domain::TypedAssetAmount;
use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox};

use crate::data::assets::{Lq, PoolNft, Reward, Tmp, VirtLq};
use crate::data::bundle::{StakingBundle, StakingBundleProto};
use crate::data::context::ExecutionContext;
use crate::data::executor::ExecutorOutput;
use crate::data::funding::{DistributionFunding, DistributionFundingProto};
use crate::data::order::{Deposit, Redeem};
use crate::data::redeemer::{DepositOutput, RedeemOutput, RewardOutput};
use crate::data::{PoolId, PoolStateId};
use crate::ergo::{NanoErg, MAX_VALUE};
use crate::validators::pool_validator;

#[derive(Debug, Eq, PartialEq, Copy, Clone, Serialize, Deserialize)]
pub struct ProgramConfig {
    pub epoch_len: u32,
    pub epoch_num: u32,
    pub program_start: u32,
    pub redeem_blocks_delta: u32,
    pub program_budget: TypedAssetAmount<Reward>,
}

impl From<ProgramConfig> for Vec<i32> {
    fn from(c: ProgramConfig) -> Self {
        vec![
            c.epoch_len as i32,
            c.epoch_num as i32,
            c.program_start as i32,
            c.redeem_blocks_delta as i32,
        ]
    }
}

#[derive(Debug, Display)]
pub enum PoolOperationError {
    Permanent(PermanentError),
    Temporal(TemporalError),
}

#[derive(Debug, Display)]
pub enum PermanentError {
    LiqudityMismatch,
    ProgramExhausted,
    OrderPoisoned,
    LowExFee,
}

#[derive(Debug, Display)]
pub enum TemporalError {
    LiquidityMoveBlocked,
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
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
    pub erg_value: NanoErg,
}

impl Pool {
    pub fn pool_nft(&self) -> TypedAssetAmount<PoolNft> {
        TypedAssetAmount::new(self.pool_id.into(), MAX_VALUE)
    }

    /// Apply deposit operation to the pool.
    /// Returns pool state after deposit, new staking bundle and user output in the case of success.
    /// Returns `PoolOperationError` otherwise.
    pub fn apply_deposit(
        self,
        deposit: Deposit,
        ctx: ExecutionContext,
    ) -> Result<(Pool, StakingBundleProto, DepositOutput, ExecutorOutput), PoolOperationError> {
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
            erg_value: NanoErg::from(BoxValue::SAFE_USER_MIN),
        };
        let user_output = DepositOutput {
            bundle_key,
            redeemer_prop: deposit.redeemer_prop,
            erg_value: NanoErg::from(BoxValue::SAFE_USER_MIN),
        };
        if deposit.erg_value
            < bundle.erg_value + user_output.erg_value + NanoErg::from(BoxValue::SAFE_USER_MIN)
        {
            return Err(PoolOperationError::Permanent(PermanentError::LowExFee));
        }
        let executor_output = ExecutorOutput {
            executor_prop: ctx.executor_prop,
            erg_value: deposit.erg_value - bundle.erg_value - user_output.erg_value,
        };
        Ok((next_pool, bundle, user_output, executor_output))
    }

    /// Apply redeem operation to the pool.
    /// Returns pool state after redeem and user output in the case of success.
    /// Returns `PoolOperationError` otherwise.
    pub fn apply_redeem(
        self,
        redeem: Redeem,
        bundle: StakingBundle,
        ctx: ExecutionContext,
    ) -> Result<(Pool, RedeemOutput, ExecutorOutput), PoolOperationError> {
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
            erg_value: NanoErg::from(BoxValue::SAFE_USER_MIN),
        };
        let erg_available = redeem.erg_value + bundle.erg_value;
        if erg_available < user_output.erg_value + NanoErg::from(BoxValue::SAFE_USER_MIN) {
            return Err(PoolOperationError::Permanent(PermanentError::LowExFee));
        }
        let executor_output = ExecutorOutput {
            executor_prop: ctx.executor_prop,
            erg_value: erg_available - user_output.erg_value,
        };
        Ok((next_pool, user_output, executor_output))
    }

    /// Distribute rewards in batch.
    /// Returns state of the pool after distribution, subtracted bundlesa and reward outputs.
    pub fn distribute_rewards(
        self,
        bundles: Vec<StakingBundle>,
        funding: NonEmpty<DistributionFunding>,
    ) -> Result<
        (
            Pool,
            Vec<StakingBundle>,
            Option<DistributionFundingProto>,
            Vec<RewardOutput>,
        ),
        PoolOperationError,
    > {
        let epoch_alloc = self.epoch_alloc();
        let epoch_ix = if self.epoch_completed() {
            ((self.conf.epoch_num + 1) as u64 - (self.budget_rem.amount / epoch_alloc)) as u32
        } else {
            ((self.conf.epoch_num + 1) as u64 - (self.budget_rem.amount / epoch_alloc) - 1) as u32
        };
        let epochs_remain = self.conf.epoch_num - epoch_ix;
        let lq_reserves_0 = self.reserves_lq;
        let mut next_pool = self;
        let mut next_bundles = bundles;
        let mut reward_outputs = Vec::new();
        let mut accumulated_cost = NanoErg::from(0u64);
        for mut bundle in &mut next_bundles {
            let epochs_burned = (bundle.tmp.amount / bundle.vlq.amount).saturating_sub(epochs_remain as u64);
            if epochs_burned < 1 {
                return Err(PoolOperationError::Permanent(PermanentError::OrderPoisoned));
            }
            let reward_amt =
                (next_pool.epoch_alloc() as u128 * bundle.vlq.amount as u128 * epochs_burned as u128
                    / lq_reserves_0.amount as u128) as u64;
            let reward = TypedAssetAmount::new(next_pool.budget_rem.token_id, reward_amt);
            let reward_output = RewardOutput {
                reward,
                redeemer_prop: bundle.redeemer_prop.clone(),
                erg_value: NanoErg::from(BoxValue::SAFE_USER_MIN),
            };
            let charged_tmp_amt = bundle.tmp.amount - epochs_remain as u64 * bundle.vlq.amount;
            let charged_tmp = TypedAssetAmount::new(bundle.tmp.token_id, charged_tmp_amt);
            accumulated_cost = accumulated_cost + reward_output.erg_value;
            reward_outputs.push(reward_output);
            bundle.tmp = bundle.tmp - charged_tmp;
            next_pool.reserves_tmp = next_pool.reserves_tmp + charged_tmp;
            next_pool.budget_rem = next_pool.budget_rem - reward;
        }
        next_pool.epoch_ix = Some(epoch_ix);
        let funds_total: NanoErg = funding.iter().map(|f| f.erg_value).sum();
        let funds_remain = funds_total - accumulated_cost;
        let next_funding_box = if funds_remain >= NanoErg::from(BoxValue::SAFE_USER_MIN) {
            Some(DistributionFundingProto {
                prop: funding.head.prop,
                erg_value: funds_remain,
            })
        } else {
            None
        };
        Ok((next_pool, next_bundles, next_funding_box, reward_outputs))
    }

    fn epoch_alloc(&self) -> u64 {
        self.conf.program_budget.amount / self.conf.epoch_num as u64
    }

    fn num_epochs_remain(&self, height: u32) -> u32 {
        self.conf.epoch_num - self.current_epoch_ix(height)
    }

    fn current_epoch_ix(&self, height: u32) -> u32 {
        let cur_block_ix = height as i64 - self.conf.program_start as i64 + 1;
        let cur_epoch_ix_rem = cur_block_ix % self.conf.epoch_len as i64;
        let cur_epoch_ix_r = cur_block_ix / self.conf.epoch_len as i64;
        if cur_epoch_ix_rem == 0 && cur_epoch_ix_r == 0 {
            0
        } else {
            if cur_epoch_ix_rem > 0 {
                cur_epoch_ix_r as u32 + 1
            } else {
                cur_epoch_ix_r as u32
            }
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
                    redeem_blocks_delta: *conf.get(3)? as u32,
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
                    erg_value: bx.value.into(),
                });
            }
        }
        None
    }
}

impl IntoBoxCandidate for Pool {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        let tokens = BoxTokens::from_vec(vec![
            Token::from(self.pool_nft()),
            Token::from(self.budget_rem),
            Token::from(self.reserves_lq),
            Token::from(self.reserves_vlq),
            Token::from(self.reserves_tmp),
        ])
        .unwrap();
        let registers = NonMandatoryRegisters::try_from(
            vec![
                Constant::from(<Vec<i32>>::from(self.conf)),
                Constant::from(self.conf.program_budget.amount as i64),
                Constant::from(self.max_error as i64),
            ]
            .into_iter()
            .chain(
                self.epoch_ix
                    .map(|ix| vec![Constant::from(ix as i32)])
                    .unwrap_or(Vec::new())
                    .into_iter(),
            )
            .collect::<Vec<_>>(),
        )
        .unwrap();
        ErgoBoxCandidate {
            value: self.erg_value.into(),
            ergo_tree: pool_validator(),
            tokens: Some(tokens),
            additional_registers: registers,
            creation_height: height,
        }
    }
}

#[cfg(test)]
mod tests {
    use ergo_lib::ergo_chain_types::Digest32;
    use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
    use ergo_lib::ergotree_ir::chain::token::TokenId;
    use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
    use ergo_lib::ergotree_ir::mir::constant::Constant;
    use ergo_lib::ergotree_ir::mir::expr::Expr;
    use nonempty::nonempty;
    use rand::Rng;

    use spectrum_offchain::domain::TypedAssetAmount;

    use crate::data::bundle::StakingBundle;
    use crate::data::context::ExecutionContext;
    use crate::data::funding::DistributionFunding;
    use crate::data::order::{Deposit, Redeem};
    use crate::data::pool::{Pool, ProgramConfig};
    use crate::data::{BundleStateId, FundingId, OrderId, PoolId, PoolStateId};
    use crate::ergo::{NanoErg, MAX_VALUE};

    fn make_pool(epoch_len: u32, epoch_num: u32, program_start: u32, program_budget: u64) -> Pool {
        Pool {
            pool_id: PoolId::from(TokenId::from(random_digest())),
            state_id: PoolStateId::from(BoxId::from(random_digest())),
            budget_rem: TypedAssetAmount::new(TokenId::from(random_digest()), program_budget),
            reserves_lq: TypedAssetAmount::new(TokenId::from(random_digest()), 0),
            reserves_vlq: TypedAssetAmount::new(TokenId::from(random_digest()), MAX_VALUE),
            reserves_tmp: TypedAssetAmount::new(TokenId::from(random_digest()), MAX_VALUE),
            epoch_ix: None,
            max_error: 0,
            conf: ProgramConfig {
                epoch_len,
                epoch_num,
                program_start,
                redeem_blocks_delta: 0,
                program_budget: TypedAssetAmount::new(TokenId::from(random_digest()), program_budget),
            },
            erg_value: NanoErg::from(100000000000u64),
        }
    }

    fn random_digest() -> Digest32 {
        let mut rng = rand::thread_rng();
        Digest32::try_from(
            (0..32)
                .map(|_| (rng.gen_range(0..=256) % 256) as u8)
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn trivial_prop() -> ErgoTree {
        ErgoTree::try_from(Expr::Const(Constant::from(true))).unwrap()
    }

    #[test]
    fn early_deposit() {
        let budget = 1000000000;
        let pool = make_pool(10, 10, 10, budget);
        let deposit_lq = TypedAssetAmount::new(pool.reserves_lq.token_id, 1000);
        let deposit = Deposit {
            order_id: OrderId::from(BoxId::from(random_digest())),
            pool_id: pool.pool_id,
            redeemer_prop: trivial_prop(),
            lq: deposit_lq,
            erg_value: NanoErg::from(100000000000u64),
            expected_num_epochs: 10,
        };
        let ctx = ExecutionContext {
            height: 9,
            mintable_token_id: TokenId::from(random_digest()),
            executor_prop: trivial_prop(),
        };
        let (pool2, bundle, _output, rew) = pool.clone().apply_deposit(deposit.clone(), ctx).unwrap();
        assert_eq!(bundle.vlq, deposit.lq.coerce());
        assert_eq!(bundle.tmp.amount, pool.conf.epoch_num as u64 * deposit_lq.amount);
        assert_eq!(pool2.reserves_lq, deposit.lq);
        assert_eq!(pool2.reserves_lq - pool.reserves_lq, deposit.lq);
        assert_eq!(pool2.reserves_vlq, pool.reserves_vlq - bundle.vlq);
        assert_eq!(pool2.reserves_tmp, pool.reserves_tmp - bundle.tmp);
    }

    #[test]
    fn deposit_redeem() {
        let budget = 1000000000;
        let pool = make_pool(10, 10, 10, budget);
        let deposit = TypedAssetAmount::new(pool.reserves_lq.token_id, 1000);
        let deposit = Deposit {
            order_id: OrderId::from(BoxId::from(random_digest())),
            pool_id: pool.pool_id,
            redeemer_prop: trivial_prop(),
            lq: deposit,
            erg_value: NanoErg::from(100000000000u64),
            expected_num_epochs: 10,
        };
        let ctx = ExecutionContext {
            height: 10,
            mintable_token_id: TokenId::from(random_digest()),
            executor_prop: trivial_prop(),
        };
        let (pool2, bundle, output, rew) = pool.clone().apply_deposit(deposit.clone(), ctx.clone()).unwrap();
        let redeem = Redeem {
            order_id: OrderId::from(BoxId::from(random_digest())),
            pool_id: pool.pool_id,
            redeemer_prop: trivial_prop(),
            bundle_key: output.bundle_key,
            expected_lq: deposit.lq,
            erg_value: NanoErg::from(100000000000u64),
        };
        let (pool3, output, rew) = pool2
            .clone()
            .apply_redeem(
                redeem.clone(),
                StakingBundle::from_proto(bundle.clone(), BundleStateId::from(BoxId::from(random_digest()))),
                ctx,
            )
            .unwrap();

        assert_eq!(pool.reserves_lq, pool3.reserves_lq);
        assert_eq!(pool.reserves_vlq, pool3.reserves_vlq);
        assert_eq!(pool.reserves_tmp, pool3.reserves_tmp);
        assert_eq!(output.lq, deposit.lq);
    }

    #[test]
    fn distribute_rewards() {
        let budget = 1000000000;
        let pool = make_pool(10, 10, 10, budget);
        let deposit_amt_a = TypedAssetAmount::new(pool.reserves_lq.token_id, 1000);
        let deposit_a = Deposit {
            order_id: OrderId::from(BoxId::from(random_digest())),
            pool_id: pool.pool_id,
            redeemer_prop: trivial_prop(),
            lq: deposit_amt_a,
            erg_value: NanoErg::from(100000000000u64),
            expected_num_epochs: 10,
        };
        let deposit_disproportion = 2;
        let deposit_amt_b = TypedAssetAmount::new(
            pool.reserves_lq.token_id,
            deposit_amt_a.amount * deposit_disproportion,
        );
        let deposit_b = Deposit {
            order_id: OrderId::from(BoxId::from(random_digest())),
            pool_id: pool.pool_id,
            redeemer_prop: trivial_prop(),
            lq: deposit_amt_b,
            erg_value: NanoErg::from(100000000000u64),
            expected_num_epochs: 10,
        };
        let ctx_1 = ExecutionContext {
            height: 9,
            mintable_token_id: TokenId::from(random_digest()),
            executor_prop: trivial_prop(),
        };
        let (pool_2, bundle_a, _output_a, rew) = pool
            .clone()
            .apply_deposit(deposit_a.clone(), ctx_1.clone())
            .unwrap();
        let (pool_3, bundle_b, _output_b, rew) =
            pool_2.clone().apply_deposit(deposit_b.clone(), ctx_1).unwrap();
        let funding = nonempty![DistributionFunding {
            id: FundingId::from(BoxId::from(random_digest())),
            prop: trivial_prop(),
            erg_value: NanoErg::from(2000000000u64),
        }];
        let (_pool_4, bundles, next_funding, rewards) = pool_3
            .distribute_rewards(
                Vec::from([
                    StakingBundle::from_proto(
                        bundle_a.clone(),
                        BundleStateId::from(BoxId::from(random_digest())),
                    ),
                    StakingBundle::from_proto(
                        bundle_b.clone(),
                        BundleStateId::from(BoxId::from(random_digest())),
                    ),
                ]),
                funding,
            )
            .unwrap();
        assert_eq!(
            rewards[0].reward.amount * deposit_disproportion,
            rewards[1].reward.amount
        );
        assert_eq!(bundles[0].tmp, bundle_a.tmp - bundle_a.vlq.coerce());
        assert_eq!(bundles[1].tmp, bundle_b.tmp - bundle_b.vlq.coerce());
    }
}
