use ergo_lib::chain::transaction::{Input, Transaction, TxIoVec};
use ergo_lib::ergo_chain_types::{blake2b256_hash, Digest32};
use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, NonMandatoryRegisterId};
use ergo_lib::ergotree_ir::chain::token::TokenId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::TryExtractInto;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use type_equalities::IsEqual;

use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Has, OnChainOrder};
use spectrum_offchain::domain::TypedAssetAmount;
use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox};
use spectrum_offchain::executor::RunOrderError;

use crate::data::assets::{BundleKey, Lq};
use crate::data::bundle::StakingBundle;
use crate::data::context::ExecutionContext;
use crate::data::pool::{Pool, PoolOperationError};
use crate::data::{AsBox, BundleId, BundleStateId, OrderId, PoolId};
use crate::ergo::empty_prover_result;
use crate::executor::{ConsumeBundle, ProduceBundle, RunOrder};
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
}

impl ConsumeBundle for Compound {
    type TBundleIn = Vec<AsBox<StakingBundle>>;
}

impl ProduceBundle for Compound {
    type TBundleOut = Vec<Predicted<AsBox<StakingBundle>>>;
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
        bundles: Vec<AsBox<StakingBundle>>,
        _ctx: ExecutionContext,
    ) -> Result<
        (
            Transaction,
            Predicted<AsBox<Pool>>,
            Vec<Predicted<AsBox<StakingBundle>>>,
        ),
        RunOrderError<Self>,
    > {
        let unwrapped_bundles = bundles.clone().into_iter().map(|AsBox(_, b)| b).collect();
        match pool.distribute_rewards(unwrapped_bundles) {
            Ok((next_pool, next_bundles, rewards)) => {
                let inputs = TxIoVec::from_vec(
                    vec![pool_in]
                        .into_iter()
                        .chain(bundles.iter().map(|AsBox(bx, _)| bx.clone()))
                        .map(|bx| Input::new(bx.box_id(), empty_prover_result()))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let outputs = TxIoVec::from_vec(
                    vec![next_pool.clone().into_candidate()]
                        .into_iter()
                        .chain(next_bundles.clone().into_iter().map(|b| b.into_candidate()))
                        .chain(rewards.into_iter().map(|r| r.into_candidate()))
                        .collect(),
                )
                .unwrap();
                let tx = Transaction::new(inputs, None, outputs).unwrap();
                let next_pool_as_box = AsBox(tx.outputs.get(0).unwrap().clone(), next_pool);
                let bundle_outs = &tx.outputs.clone()[1..next_bundles.len()];
                let bundles_as_box = next_bundles
                    .into_iter()
                    .zip(Vec::from(bundle_outs).into_iter())
                    .map(|(bn, out)| Predicted(AsBox(out, bn)))
                    .collect();
                Ok((tx, Predicted(next_pool_as_box), bundles_as_box))
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
}

impl ConsumeBundle for Deposit {
    type TBundleIn = ();
}

impl ProduceBundle for Deposit {
    type TBundleOut = Predicted<AsBox<StakingBundle>>;
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
            Transaction,
            Predicted<AsBox<Pool>>,
            Predicted<AsBox<StakingBundle>>,
        ),
        RunOrderError<Self>,
    > {
        let AsBox(self_in, self_order) = self.clone();
        match pool.apply_deposit(self_order, ctx) {
            Ok((next_pool, bundle_proto, user_out)) => {
                let inputs = TxIoVec::from_vec(
                    vec![pool_in, self_in]
                        .iter()
                        .map(|bx| Input::new(bx.box_id(), empty_prover_result()))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let outputs = TxIoVec::from_vec(vec![
                    next_pool.clone().into_candidate(),
                    bundle_proto.clone().into_candidate(),
                    user_out.into_candidate(),
                ])
                .unwrap();
                let tx = Transaction::new(inputs, None, outputs).unwrap();
                let next_pool_as_box = AsBox(tx.outputs.get(0).unwrap().clone(), next_pool);
                let bundle_box = tx.outputs.get(1).unwrap().clone();
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
                        .get_constant(7)
                        .ok()??
                        .v
                        .try_extract_into::<Vec<u8>>()
                        .ok()?,
                )
                .ok()?;
                let redeemer_prop = ErgoTree::sigma_parse_bytes(
                    &*bx.ergo_tree
                        .get_constant(2)
                        .ok()??
                        .v
                        .try_extract_into::<Vec<u8>>()
                        .ok()?,
                )
                .ok()?;
                let lq = TypedAssetAmount::<Lq>::from_token(tokens.get(0)?.clone());
                return Some(Deposit {
                    order_id,
                    pool_id: PoolId::from(TokenId::from(pool_id)),
                    redeemer_prop,
                    lq,
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
}

impl ConsumeBundle for Redeem {
    type TBundleIn = AsBox<StakingBundle>;
}

impl ProduceBundle for Redeem {
    type TBundleOut = ();
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
        _ctx: ExecutionContext,
    ) -> Result<(Transaction, Predicted<AsBox<Pool>>, ()), RunOrderError<Self>> {
        let AsBox(self_in, self_order) = self.clone();
        match pool.apply_redeem(self_order, bundle) {
            Ok((next_pool, user_out)) => {
                let inputs = TxIoVec::from_vec(
                    vec![pool_in, bundle_in, self_in]
                        .iter()
                        .map(|bx| Input::new(bx.box_id(), empty_prover_result()))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let outputs = TxIoVec::from_vec(vec![
                    next_pool.clone().into_candidate(),
                    user_out.into_candidate(),
                ])
                .unwrap();
                let tx = Transaction::new(inputs, None, outputs).unwrap();
                let next_pool_as_box = AsBox(tx.outputs.get(0).unwrap().clone(), next_pool);
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
