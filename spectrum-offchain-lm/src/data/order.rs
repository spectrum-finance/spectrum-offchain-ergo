use ergo_lib::chain::transaction::TxIoVec;
use ergo_lib::ergo_chain_types::{blake2b256_hash, Digest32};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ContextExtension;
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, NonMandatoryRegisterId};
use ergo_lib::ergotree_ir::chain::token::TokenId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::{Constant, TryExtractInto};
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use indexmap::IndexMap;
use itertools::Itertools;
use nonempty::NonEmpty;
use type_equalities::IsEqual;

use spectrum_offchain::backlog::data::{OrderWeight, Weighted};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Has, OnChainOrder};
use spectrum_offchain::domain::TypedAssetAmount;
use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox};
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::transaction::{TransactionCandidate, UnsignedTransactionOps};

use crate::data::assets::{BundleKey, Lq};
use crate::data::bundle::StakingBundle;
use crate::data::context::ExecutionContext;
use crate::data::funding::DistributionFunding;
use crate::data::pool::{Pool, PoolOperationError};
use crate::data::{AsBox, BundleId, BundleStateId, FundingId, OrderId, PoolId};
use crate::ergo::NanoErg;
use crate::executor::{ConsumeExtra, ProduceExtra, RunOrder};
use crate::validators::{deposit_validator_temp, redeem_validator_temp};

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Compound {
    pub pool_id: PoolId,
    pub epoch_ix: u32,
    pub queue_ix: usize,
    pub stakers: Vec<BundleId>,
}

impl Compound {
    pub fn order_id(&self) -> OrderId {
        let preimage = format!("{}{}{}", self.pool_id, self.epoch_ix, self.queue_ix);
        OrderId::from(blake2b256_hash(preimage.as_bytes()))
    }

    pub fn estimated_min_value(&self) -> NanoErg {
        NanoErg::from(self.stakers.len() as u64 * BoxValue::SAFE_USER_MIN.as_u64())
    }
}

impl ConsumeExtra for Compound {
    type TExtraIn = (Vec<AsBox<StakingBundle>>, NonEmpty<AsBox<DistributionFunding>>);
}

impl ProduceExtra for Compound {
    type TExtraOut = (
        Vec<Predicted<AsBox<StakingBundle>>>,
        Option<Predicted<AsBox<DistributionFunding>>>,
    );
}

impl OnChainOrder for Compound {
    type TOrderId = OrderId;
    type TEntityId = PoolId;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.order_id()
    }

    fn get_entity_ref(&self) -> Self::TEntityId {
        self.pool_id
    }
}

impl RunOrder for Compound {
    fn try_run(
        self,
        AsBox(pool_in, pool): AsBox<Pool>,
        (bundles, funding): (Vec<AsBox<StakingBundle>>, NonEmpty<AsBox<DistributionFunding>>),
        ctx: ExecutionContext,
    ) -> Result<
        (
            TransactionCandidate,
            Predicted<AsBox<Pool>>,
            (
                Vec<Predicted<AsBox<StakingBundle>>>,
                Option<Predicted<AsBox<DistributionFunding>>>,
            ),
        ),
        RunOrderError<Self>,
    > {
        let unwrapped_bundles = bundles.clone().into_iter().map(|AsBox(_, b)| b).collect();
        match pool.distribute_rewards(unwrapped_bundles, funding.clone().map(|AsBox(_, f)| f)) {
            Ok((next_pool, next_bundles, next_funding, rewards)) => {
                let outputs = TxIoVec::from_vec(
                    vec![next_pool.clone().into_candidate(ctx.height)]
                        .into_iter()
                        .chain(
                            next_funding
                                .clone()
                                .map(|nf| vec![nf.clone().into_candidate(ctx.height)])
                                .unwrap_or(Vec::new()),
                        )
                        .chain(
                            next_bundles
                                .clone()
                                .into_iter()
                                .map(|b| b.into_candidate(ctx.height)),
                        )
                        .chain(rewards.into_iter().map(|r| r.into_candidate(ctx.height)))
                        .collect(),
                )
                .unwrap();
                let inputs = TxIoVec::from_vec(
                    vec![pool_in]
                        .into_iter()
                        .chain(funding.map(|AsBox(i, _)| i))
                        .map(|bx| (bx, ContextExtension::empty()))
                        .chain(bundles.iter().map(|AsBox(bx, _)| bx.clone()).map(|bx| {
                            let redeemer_prop = ErgoTree::sigma_parse_bytes(
                                &*bx.additional_registers
                                    .get(NonMandatoryRegisterId::R4)
                                    .unwrap()
                                    .as_option_constant()
                                    .cloned()
                                    .unwrap()
                                    .try_extract_into::<Vec<u8>>()
                                    .unwrap(),
                            )
                            .unwrap();
                            let (redeemer_out_ix, _) = outputs
                                .iter()
                                .find_position(|o| o.ergo_tree == redeemer_prop)
                                .expect("Redeemer out not found");
                            let (succ_ix, _) = outputs
                                .iter()
                                .find_position(|o| o.additional_registers == bx.additional_registers)
                                .expect("Successor out not found");
                            let mut constants = IndexMap::new();
                            constants.insert(0u8, Constant::from(redeemer_out_ix as i32));
                            constants.insert(1u8, Constant::from(succ_ix as i32));
                            (bx, ContextExtension { values: constants })
                        }))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let tx = TransactionCandidate::new(inputs, None, outputs);
                let outputs = tx.clone().into_tx_without_proofs().outputs;
                let next_pool_as_box = AsBox(outputs.get(0).unwrap().clone(), next_pool);
                let bundle_outs = &outputs.clone()[2..next_bundles.len()];
                let bundles_as_box = next_bundles
                    .into_iter()
                    .zip(Vec::from(bundle_outs).into_iter())
                    .map(|(bn, out)| Predicted(AsBox(out, bn)))
                    .collect();
                let next_funding_as_box = next_funding.map(|nf| {
                    let out = outputs.get(1).unwrap().clone();
                    let funding_id = FundingId::from(out.box_id());
                    Predicted(AsBox(out, nf.complete(funding_id)))
                });
                Ok((
                    tx,
                    Predicted(next_pool_as_box),
                    (bundles_as_box, next_funding_as_box),
                ))
            }
            Err(PoolOperationError::Permanent(e)) => Err(RunOrderError::Fatal(format!("{}", e), self)),
            Err(PoolOperationError::Temporal(e)) => Err(RunOrderError::NonFatal(format!("{}", e), self)),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Deposit {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub redeemer_prop: ErgoTree,
    pub lq: TypedAssetAmount<Lq>,
    pub erg_value: NanoErg,
    pub expected_num_epochs: u32,
}

impl ConsumeExtra for Deposit {
    type TExtraIn = ();
}

impl ProduceExtra for Deposit {
    type TExtraOut = Predicted<AsBox<StakingBundle>>;
}

impl OnChainOrder for Deposit {
    type TOrderId = OrderId;
    type TEntityId = PoolId;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.order_id
    }

    fn get_entity_ref(&self) -> Self::TEntityId {
        self.pool_id
    }
}

impl RunOrder for AsBox<Deposit> {
    fn try_run(
        self,
        AsBox(pool_in, pool): AsBox<Pool>,
        _bundle: (),
        ctx: ExecutionContext,
    ) -> Result<
        (
            TransactionCandidate,
            Predicted<AsBox<Pool>>,
            Predicted<AsBox<StakingBundle>>,
        ),
        RunOrderError<Self>,
    > {
        let AsBox(self_in, self_order) = self.clone();
        match pool.apply_deposit(self_order, ctx.clone()) {
            Ok((next_pool, bundle_proto, user_out, executor_out)) => {
                let inputs = TxIoVec::from_vec(
                    vec![pool_in, self_in]
                        .into_iter()
                        .map(|bx| (bx, ContextExtension::empty()))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let outputs = TxIoVec::from_vec(vec![
                    next_pool.clone().into_candidate(ctx.height),
                    bundle_proto.clone().into_candidate(ctx.height),
                    user_out.into_candidate(ctx.height),
                    executor_out.into_candidate(ctx.height),
                ])
                .unwrap();
                let tx = TransactionCandidate::new(inputs, None, outputs);
                let outputs = tx.clone().into_tx_without_proofs().outputs;
                let next_pool_as_box = AsBox(outputs.get(0).unwrap().clone(), next_pool);
                let bundle_box = outputs.get(1).unwrap().clone();
                let bundle = bundle_proto.finalize(BundleStateId::from(bundle_box.box_id()));
                let bundle_as_box = AsBox(bundle_box, bundle);
                Ok((tx, Predicted(next_pool_as_box), Predicted(bundle_as_box)))
            }
            Err(PoolOperationError::Temporal(te)) => Err(RunOrderError::NonFatal(format!("{}", te), self)),
            Err(PoolOperationError::Permanent(pe)) => Err(RunOrderError::Fatal(format!("{}", pe), self)),
        }
    }
}

impl TryFromBox for Deposit {
    fn try_from_box(bx: ErgoBox) -> Option<Deposit> {
        if let Some(ref tokens) = bx.tokens {
            if bx.ergo_tree.template_bytes().ok()? == deposit_validator_temp() && tokens.len() == 1 {
                let order_id = OrderId::from(bx.box_id());
                let pool_id = Digest32::try_from(
                    bx.ergo_tree
                        .get_constant(1)
                        .ok()??
                        .v
                        .try_extract_into::<Vec<u8>>()
                        .ok()?,
                )
                .ok()?;
                let redeemer_prop = ErgoTree::sigma_parse_bytes(
                    &*bx.ergo_tree
                        .get_constant(3)
                        .ok()??
                        .v
                        .try_extract_into::<Vec<u8>>()
                        .ok()?,
                )
                .ok()?;
                let expected_num_epochs = bx
                    .ergo_tree
                    .get_constant(13)
                    .ok()??
                    .v
                    .try_extract_into::<i32>()
                    .ok()?;
                let lq = TypedAssetAmount::<Lq>::from_token(tokens.get(0)?.clone());
                return Some(Deposit {
                    order_id,
                    pool_id: PoolId::from(TokenId::from(pool_id)),
                    redeemer_prop,
                    lq,
                    erg_value: bx.value.into(),
                    expected_num_epochs: expected_num_epochs as u32,
                });
            }
        }
        None
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Redeem {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub redeemer_prop: ErgoTree,
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub expected_lq: TypedAssetAmount<Lq>,
    pub erg_value: NanoErg,
}

impl ConsumeExtra for Redeem {
    type TExtraIn = AsBox<StakingBundle>;
}

impl ProduceExtra for Redeem {
    type TExtraOut = ();
}

impl OnChainOrder for Redeem {
    type TOrderId = OrderId;
    type TEntityId = PoolId;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.order_id
    }

    fn get_entity_ref(&self) -> Self::TEntityId {
        self.pool_id
    }
}

impl RunOrder for AsBox<Redeem> {
    fn try_run(
        self,
        AsBox(pool_in, pool): AsBox<Pool>,
        AsBox(bundle_in, bundle): AsBox<StakingBundle>,
        ctx: ExecutionContext,
    ) -> Result<(TransactionCandidate, Predicted<AsBox<Pool>>, ()), RunOrderError<Self>> {
        let AsBox(self_in, self_order) = self.clone();
        match pool.apply_redeem(self_order, bundle, ctx.clone()) {
            Ok((next_pool, user_out, executor_out)) => {
                let inputs = TxIoVec::from_vec(
                    vec![pool_in, bundle_in, self_in]
                        .into_iter()
                        .map(|bx| (bx, ContextExtension::empty()))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let outputs = TxIoVec::from_vec(vec![
                    next_pool.clone().into_candidate(ctx.height),
                    user_out.into_candidate(ctx.height),
                    executor_out.into_candidate(ctx.height),
                ])
                .unwrap();
                let tx = TransactionCandidate::new(inputs, None, outputs);
                let outputs = tx.clone().into_tx_without_proofs().outputs;
                let next_pool_as_box = AsBox(outputs.get(0).unwrap().clone(), next_pool);
                Ok((tx, Predicted(next_pool_as_box), ()))
            }
            Err(PoolOperationError::Temporal(te)) => Err(RunOrderError::NonFatal(format!("{}", te), self)),
            Err(PoolOperationError::Permanent(pe)) => Err(RunOrderError::Fatal(format!("{}", pe), self)),
        }
    }
}

impl TryFromBox for Redeem {
    fn try_from_box(bx: ErgoBox) -> Option<Redeem> {
        if let Some(ref tokens) = bx.tokens {
            if bx.ergo_tree.template_bytes().ok()? == redeem_validator_temp() && tokens.len() == 1 {
                let order_id = OrderId::from(bx.box_id());
                let pool_id = PoolId::from(TokenId::from(
                    Digest32::try_from(
                        bx.additional_registers
                            .get(NonMandatoryRegisterId::R4)?
                            .as_option_constant()
                            .map(|c| c.clone().try_extract_into::<Vec<u8>>())?
                            .ok()?,
                    )
                    .ok()?,
                ));
                let redeemer_prop = ErgoTree::sigma_parse_bytes(
                    &*bx.ergo_tree
                        .get_constant(2)
                        .ok()??
                        .v
                        .try_extract_into::<Vec<u8>>()
                        .ok()?,
                )
                .ok()?;
                let bundle_key = TypedAssetAmount::<BundleKey>::from_token(tokens.get(0)?.clone());
                let expected_lq_id = TokenId::from(
                    Digest32::try_from(
                        bx.ergo_tree
                            .get_constant(3)
                            .ok()??
                            .v
                            .try_extract_into::<Vec<u8>>()
                            .ok()?,
                    )
                    .ok()?,
                );
                let expected_lq_amt = bx
                    .ergo_tree
                    .get_constant(4)
                    .ok()??
                    .v
                    .try_extract_into::<i64>()
                    .ok()? as u64;
                let expected_lq = TypedAssetAmount::new(expected_lq_id, expected_lq_amt);
                return Some(Redeem {
                    order_id,
                    pool_id,
                    redeemer_prop,
                    bundle_key,
                    expected_lq,
                    erg_value: bx.value.into(),
                });
            }
        }
        None
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Order {
    Deposit(AsBox<Deposit>),
    Redeem(AsBox<Redeem>),
    Compound(Compound),
}

impl OnChainOrder for Order {
    type TOrderId = OrderId;
    type TEntityId = PoolId;

    fn get_self_ref(&self) -> Self::TOrderId {
        match self {
            Order::Deposit(AsBox(_, deposit)) => deposit.order_id,
            Order::Redeem(AsBox(_, redeem)) => redeem.order_id,
            Order::Compound(compound) => compound.order_id(),
        }
    }

    fn get_entity_ref(&self) -> Self::TEntityId {
        match self {
            Order::Deposit(AsBox(_, deposit)) => deposit.pool_id,
            Order::Redeem(AsBox(_, redeem)) => redeem.pool_id,
            Order::Compound(compound) => compound.pool_id,
        }
    }
}

impl Has<Vec<BundleId>> for Order {
    fn get<U: IsEqual<Vec<BundleId>>>(&self) -> Vec<BundleId> {
        match self {
            Order::Redeem(AsBox(_, redeem)) => vec![BundleId::from(redeem.bundle_key.token_id)],
            Order::Compound(compound) => compound.clone().stakers,
            _ => Vec::new(),
        }
    }
}

impl TryFromBox for Order {
    fn try_from_box(bx: ErgoBox) -> Option<Order> {
        Deposit::try_from_box(bx.clone())
            .map(|d| Order::Deposit(AsBox(bx.clone(), d)))
            .or_else(|| Redeem::try_from_box(bx.clone()).map(|r| Order::Redeem(AsBox(bx, r))))
    }
}

const COMPOUND_BASE_WEIGHT: u64 = 1_000_000_000_000;

impl Weighted for Order {
    fn weight(&self) -> OrderWeight {
        match self {
            Order::Deposit(AsBox(_, deposit)) => OrderWeight::from(<u64>::from(deposit.erg_value)),
            Order::Redeem(AsBox(_, redeem)) => OrderWeight::from(<u64>::from(redeem.erg_value)),
            Order::Compound(compound) => {
                OrderWeight::from(compound.epoch_ix as u64 * compound.queue_ix as u64 * COMPOUND_BASE_WEIGHT)
            }
        }
    }
}
