use std::hash::{Hash, Hasher};

use ergo_lib::chain::transaction::TxIoVec;
use ergo_lib::ergo_chain_types::{blake2b256_hash, Digest32};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ContextExtension;
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, NonMandatoryRegisterId};
use ergo_lib::ergotree_ir::chain::token::TokenId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::{Constant, TryExtractInto};
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::{ProveDlog, SigmaProp};
use ergo_lib::wallet::miner_fee::MINERS_FEE_BASE16_BYTES;
use indexmap::IndexMap;
use itertools::Itertools;
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
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
use crate::ergo::{default_sigma_prop_tree, NanoErg};
use crate::executor::{ConsumeExtra, ProduceExtra, RunOrder};
use crate::validators::{DEPOSIT_TEMPLATE, REDEEM_TEMPLATE};

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
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
            Ok((next_pool, next_bundles, next_funding, rewards, miner_out)) => {
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
                        .chain(vec![miner_out.into_candidate(ctx.height)])
                        .collect(),
                )
                .unwrap();
                let inputs = TxIoVec::from_vec(
                    vec![pool_in]
                        .into_iter()
                        .chain(funding.map(|AsBox(i, _)| i))
                        .map(|bx| (bx, ContextExtension::empty()))
                        .chain(bundles.iter().map(|AsBox(bx, bn)| {
                            let bx = bx.clone();
                            let redeemer_prop = default_sigma_prop_tree(bn.redeemer_prop.clone());
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
                let bun_init_ix = if next_funding.is_some() { 2 } else { 1 };
                let bundle_outs = &outputs.clone()[bun_init_ix..bun_init_ix + next_bundles.len()];
                let bundles_as_box = next_bundles
                    .into_iter()
                    .zip(Vec::from(bundle_outs).into_iter())
                    .map(|(bn, out)| Predicted(AsBox(out, bn)))
                    .collect();
                let next_funding_as_box = next_funding.map(|nf| {
                    let out = outputs.get(1).unwrap().clone();
                    let funding_id = FundingId::from(out.box_id());
                    Predicted(AsBox(out, nf.finalize(funding_id)))
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

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
#[serde(from = "RawDeposit")]
#[serde(into = "RawDeposit")]
pub struct Deposit {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub redeemer_prop: SigmaProp,
    pub bundle_prop_hash: Digest32,
    pub max_miner_fee: i64,
    pub lq: TypedAssetAmount<Lq>,
    pub erg_value: NanoErg,
    pub expected_num_epochs: u32,
}

impl From<RawDeposit> for Deposit {
    fn from(rd: RawDeposit) -> Self {
        Self {
            order_id: rd.order_id,
            pool_id: rd.pool_id,
            redeemer_prop: SigmaProp::from(
                ProveDlog::try_from(ErgoTree::sigma_parse_bytes(&rd.redeemer_prop_raw).unwrap()).unwrap(),
            ),
            bundle_prop_hash: rd.bundle_prop_hash,
            max_miner_fee: rd.max_miner_fee,
            lq: TypedAssetAmount::new(rd.lq.0, rd.lq.1),
            erg_value: NanoErg::from(rd.erg_value),
            expected_num_epochs: rd.expected_num_epochs,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct RawDeposit {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub redeemer_prop_raw: Vec<u8>,
    pub bundle_prop_hash: Digest32,
    pub max_miner_fee: i64,
    pub lq: (TokenId, u64),
    pub erg_value: u64,
    pub expected_num_epochs: u32,
}

impl From<Deposit> for RawDeposit {
    fn from(d: Deposit) -> Self {
        Self {
            order_id: d.order_id,
            pool_id: d.pool_id,
            redeemer_prop_raw: d.redeemer_prop.prop_bytes().unwrap(),
            bundle_prop_hash: d.bundle_prop_hash,
            max_miner_fee: d.max_miner_fee,
            lq: (d.lq.token_id, d.lq.amount),
            erg_value: d.erg_value.into(),
            expected_num_epochs: d.expected_num_epochs,
        }
    }
}

impl Hash for Deposit {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.order_id.hash(state);
        self.pool_id.hash(state);
        state.write(&*self.redeemer_prop.prop_bytes().unwrap());
        self.bundle_prop_hash.hash(state);
        self.max_miner_fee.hash(state);
        self.lq.hash(state);
        self.erg_value.hash(state);
        self.expected_num_epochs.hash(state);
    }
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
            Ok((next_pool, bundle_proto, user_out, executor_out, miner_out)) => {
                let inputs = TxIoVec::from_vec(
                    vec![pool_in, self_in]
                        .into_iter()
                        .map(|bx| (bx, ContextExtension::empty()))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let outputs = TxIoVec::from_vec(vec![
                    next_pool.clone().into_candidate(ctx.height),
                    user_out.into_candidate(ctx.height),
                    bundle_proto.clone().into_candidate(ctx.height),
                    executor_out.into_candidate(ctx.height),
                    miner_out.into_candidate(ctx.height),
                ])
                .unwrap();
                let tx = TransactionCandidate::new(inputs, None, outputs);
                let outputs = tx.clone().into_tx_without_proofs().outputs;
                let next_pool_as_box = AsBox(outputs.get(0).unwrap().clone(), next_pool);
                let bundle_box = outputs.get(2).unwrap().clone();
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
            if bx.ergo_tree.template_bytes().ok()? == *DEPOSIT_TEMPLATE && tokens.len() == 1 {
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
                let redeemer_prop = SigmaProp::from(
                    ProveDlog::try_from(
                        ErgoTree::sigma_parse_bytes(
                            &*bx.ergo_tree
                                .get_constant(3)
                                .ok()??
                                .v
                                .try_extract_into::<Vec<u8>>()
                                .ok()?,
                        )
                        .ok()?,
                    )
                    .ok()?,
                );
                let bundle_prop_hash = bx
                    .ergo_tree
                    .get_constant(10)
                    .ok()??
                    .v
                    .try_extract_into::<Digest32>()
                    .ok()?;
                let max_miner_fee = bx
                    .ergo_tree
                    .get_constant(21)
                    .ok()??
                    .v
                    .try_extract_into::<i64>()
                    .ok()?;
                let expected_num_epochs = bx
                    .ergo_tree
                    .get_constant(14)
                    .ok()??
                    .v
                    .try_extract_into::<i32>()
                    .ok()?;
                let miner_prop_bytes = bx
                    .ergo_tree
                    .get_constant(18)
                    .ok()??
                    .v
                    .try_extract_into::<Vec<u8>>()
                    .ok()?;
                if miner_prop_bytes == base16::decode(MINERS_FEE_BASE16_BYTES).unwrap() {
                    let lq = TypedAssetAmount::<Lq>::from_token(tokens.get(0)?.clone());
                    return Some(Deposit {
                        order_id,
                        pool_id: PoolId::from(TokenId::from(pool_id)),
                        redeemer_prop,
                        bundle_prop_hash,
                        max_miner_fee,
                        lq,
                        erg_value: bx.value.into(),
                        expected_num_epochs: expected_num_epochs as u32,
                    });
                }
            }
        }
        None
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
#[serde(from = "RawRedeem")]
#[serde(into = "RawRedeem")]
pub struct Redeem {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub redeemer_prop: ErgoTree,
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub expected_lq: TypedAssetAmount<Lq>,
    pub max_miner_fee: i64,
    pub erg_value: NanoErg,
}

impl From<RawRedeem> for Redeem {
    fn from(rr: RawRedeem) -> Self {
        Self {
            order_id: rr.order_id,
            pool_id: rr.pool_id,
            redeemer_prop: ErgoTree::sigma_parse_bytes(&*rr.redeemer_prop_bytes).unwrap(),
            bundle_key: TypedAssetAmount::new(rr.bundle_key.0, rr.bundle_key.1),
            expected_lq: TypedAssetAmount::new(rr.expected_lq.0, rr.expected_lq.1),
            max_miner_fee: rr.max_miner_fee,
            erg_value: NanoErg::from(rr.erg_value),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct RawRedeem {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub redeemer_prop_bytes: Vec<u8>,
    pub bundle_key: (TokenId, u64),
    pub expected_lq: (TokenId, u64),
    pub max_miner_fee: i64,
    pub erg_value: u64,
}

impl From<Redeem> for RawRedeem {
    fn from(r: Redeem) -> Self {
        Self {
            order_id: r.order_id,
            pool_id: r.pool_id,
            redeemer_prop_bytes: r.redeemer_prop.sigma_serialize_bytes().unwrap(),
            bundle_key: (r.bundle_key.token_id, r.bundle_key.amount),
            expected_lq: (r.expected_lq.token_id, r.expected_lq.amount),
            max_miner_fee: r.max_miner_fee,
            erg_value: r.erg_value.into(),
        }
    }
}

impl Hash for Redeem {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.order_id.hash(state);
        self.pool_id.hash(state);
        state.write(&*self.redeemer_prop.sigma_serialize_bytes().unwrap());
        self.max_miner_fee.hash(state);
        self.bundle_key.hash(state);
        self.expected_lq.hash(state);
        self.erg_value.hash(state);
    }
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
            Ok((next_pool, user_out, executor_out, miner_out)) => {
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
                    miner_out.into_candidate(ctx.height),
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
            if bx.ergo_tree.template_bytes().ok()? == *REDEEM_TEMPLATE && tokens.len() == 1 {
                let order_id = OrderId::from(bx.box_id());
                let pool_id = PoolId::from(TokenId::from(
                    Digest32::try_from(
                        bx.get_register(NonMandatoryRegisterId::R4.into())?
                            .v
                            .try_extract_into::<Vec<u8>>()
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
                let miner_prop_bytes = bx
                    .ergo_tree
                    .get_constant(6)
                    .ok()??
                    .v
                    .try_extract_into::<Vec<u8>>()
                    .ok()?;
                assert_eq!(miner_prop_bytes, base16::decode(MINERS_FEE_BASE16_BYTES).unwrap());
                let max_miner_fee = bx
                    .ergo_tree
                    .get_constant(9)
                    .ok()??
                    .v
                    .try_extract_into::<i64>()
                    .ok()?;
                return Some(Redeem {
                    order_id,
                    pool_id,
                    redeemer_prop,
                    bundle_key,
                    expected_lq,
                    max_miner_fee,
                    erg_value: bx.value.into(),
                });
            }
        }
        None
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use ergo_chain_sync::client::node::ErgoNodeHttpClient;
    use ergo_chain_sync::client::types::Url;
    use ergo_lib::ergo_chain_types::Digest32;
    use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
    use ergo_lib::ergotree_ir::ergo_tree::{ErgoTree, ErgoTreeHeader};
    use ergo_lib::ergotree_ir::mir::constant::{Constant, Literal};
    use ergo_lib::ergotree_ir::mir::expr::Expr;
    use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
    use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::{
        ProveDlog, SigmaBoolean, SigmaProofOfKnowledgeTree, SigmaProp,
    };

    use ergo_lib::ergotree_ir::types::stype::SType;
    use ergo_lib::wallet::miner_fee::MINERS_FEE_BASE16_BYTES;
    use isahc::prelude::Configurable;
    use isahc::HttpClient;
    use sigma_test_util::force_any_val;
    use spectrum_offchain::event_sink::handlers::types::TryFromBox;
    use spectrum_offchain::network::ErgoNetwork;

    use crate::data::context::ExecutionContext;
    use crate::data::order::{Deposit, Order};
    use crate::data::pool::Pool;
    use crate::data::AsBox;
    use crate::executor::RunOrder;
    use crate::prover::{SigmaProver, Wallet};

    fn trivial_prop() -> ErgoTree {
        ErgoTree::try_from(Expr::Const(Constant::from(true))).unwrap()
    }

    #[test]
    fn parse_order() {
        let sample_json = r#"{
            "boxId": "006c87fcd896d414cdb44497f9e6f508e3b7939cb791a710b193dd854c10145b",
            "value": 2750000,
            "ergoTree": "198d041604000e2048ad28d9bb55e1da36d27c655a84279ff25d889063255d3f774ff926a370437004020e240008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec0404040008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec040005fcffffffffffffffff0104000e20599e30a83bc971f75582f2581f0633eebfe936b95d956ed103cbec520d804386040604000408041c0402050204040e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a5730405000500058092f401d808d601b2a4730000d602db63087201d6037301d604b2a5730200d6057303d606c57201d607b2a5730400d6088cb2db6308a773050002eb027306d1ededed938cb27202730700017203ed93c27204720593860272067308b2db63087204730900ededededed93cbc27207730a93d0e4c672070408720593e4c67207050e72039386028cb27202730b00017208b2db63087207730c009386028cb27202730d00019c72087e730e05b2db63087207730f0093860272067310b2db6308720773110090b0ada5d90109639593c272097312c1720973137314d90109599a8c7209018c7209027315",
            "assets": [
                {
                    "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                    "amount": 71
                }
            ],
            "creationHeight": 921698,
            "additionalRegisters": {},
            "transactionId": "4aaa737e4ce515d0dc5a27e3fecf24702f7c487cb873c0fbd0416526a1cb74c0",
            "index": 0
        }"#;
        let bx: ErgoBox = serde_json::from_str(sample_json).unwrap();
        let res = Order::try_from_box(bx);
        println!("{:?}", res);
        assert!(res.is_some())
    }

    #[test]
    fn run_deposit() {
        let deposit_json = r#"{
            "boxId": "006c87fcd896d414cdb44497f9e6f508e3b7939cb791a710b193dd854c10145b",
            "value": 2750000,
            "ergoTree": "198d041604000e2048ad28d9bb55e1da36d27c655a84279ff25d889063255d3f774ff926a370437004020e240008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec0404040008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec040005fcffffffffffffffff0104000e20599e30a83bc971f75582f2581f0633eebfe936b95d956ed103cbec520d804386040604000408041c0402050204040e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a5730405000500058092f401d808d601b2a4730000d602db63087201d6037301d604b2a5730200d6057303d606c57201d607b2a5730400d6088cb2db6308a773050002eb027306d1ededed938cb27202730700017203ed93c27204720593860272067308b2db63087204730900ededededed93cbc27207730a93d0e4c672070408720593e4c67207050e72039386028cb27202730b00017208b2db63087207730c009386028cb27202730d00019c72087e730e05b2db63087207730f0093860272067310b2db6308720773110090b0ada5d90109639593c272097312c1720973137314d90109599a8c7209018c7209027315",
            "assets": [
                {
                    "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                    "amount": 71
                }
            ],
            "creationHeight": 921698,
            "additionalRegisters": {},
            "transactionId": "4aaa737e4ce515d0dc5a27e3fecf24702f7c487cb873c0fbd0416526a1cb74c0",
            "index": 0
        }"#;
        let pool_box: ErgoBox = serde_json::from_str(POOL_JSON).unwrap();
        let pool = <AsBox<Pool>>::try_from_box(pool_box).unwrap();
        let deposit_box: ErgoBox = serde_json::from_str(deposit_json).unwrap();
        let deposit = <AsBox<Deposit>>::try_from_box(deposit_box).unwrap();

        let ec = ExecutionContext {
            height: 921700,
            mintable_token_id: pool.0.box_id().into(),
            executor_prop: trivial_prop(),
        };

        let res = deposit.clone().try_run(pool, (), ec);

        println!("{:?}", res);
        assert!(res.is_ok());

        let prover = Wallet::trivial(Vec::new());

        //println!("{}", serde_json::to_string(&res.unwrap().0.into_tx_without_proofs()).unwrap());

        let signed_tx = prover.sign(res.unwrap().0);

        assert!(signed_tx.is_ok());
    }

    #[tokio::test]
    async fn submit_deposit_tx() {
        let deposit_json = r#"{
            "boxId": "006d1f580237bb3d482c74ce2eadac2676e32066afc6e9f58ceb5fd907483b20",
            "value": 2750000,
            "ergoTree": "1987041604000e20020202020202020202020202020202020202020202020202020202020202020204020e2000000000000000000000000000000000000000000000000000000000000000000404040008cd02217daf90deb73bdf8b6709bb42093fdfaff6573fd47b630e2d3fdd4a8193a74d040005fcffffffffffffffff0104000e20010101010101010101010101010101010101010101010101010101010101010104060400040804140402050204040e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a573040500050005a09c01d808d601b2a4730000d602db63087201d6037301d604b2a5730200d6057303d606c57201d607b2a5730400d6088cb2db6308a773050002eb027306d1ededed938cb27202730700017203ed93c27204720593860272067308b2db63087204730900ededededed93cbc27207730a93e4c67207040e720593e4c67207050e72039386028cb27202730b00017208b2db63087207730c009386028cb27202730d00019c72087e730e05b2db63087207730f0093860272067310b2db6308720773110090b0ada5d90109639593c272097312c1720973137314d90109599a8c7209018c7209027315",
            "assets": [
                {
                    "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                    "amount": 71
                }
            ],
            "creationHeight": 921698,
            "additionalRegisters": {},
            "transactionId": "4aaa737e4ce515d0dc5a27e3fecf24702f7c487cb873c0fbd0416526a1cb74c0",
            "index": 0
        }"#;
        let pool_box: ErgoBox = serde_json::from_str(POOL_JSON).unwrap();
        let pool = <AsBox<Pool>>::try_from_box(pool_box).unwrap();
        let mut deposit_box: ErgoBox = serde_json::from_str(deposit_json).unwrap();

        let pdlog = force_any_val::<ProveDlog>();
        let c = Constant {
            tpe: SType::SSigmaProp,
            v: Literal::SigmaProp(Box::new(SigmaProp::from(SigmaBoolean::ProofOfKnowledge(
                SigmaProofOfKnowledgeTree::ProveDlog(pdlog),
            )))),
        };
        let etree = ErgoTree::new(ErgoTreeHeader::v1(false), &Expr::Const(c)).unwrap();
        let expr = deposit_box.ergo_tree.clone().proposition().unwrap();
        let tree = ErgoTree::new(ErgoTreeHeader::v1(true), &expr)
            .unwrap()
            .with_constant(1, <Vec<u8>>::from(force_any_val::<Digest32>()).into())
            .unwrap()
            .with_constant(3, etree.sigma_serialize_bytes().unwrap().into())
            .unwrap()
            .with_constant(10, force_any_val::<Digest32>().into())
            .unwrap()
            .with_constant(21, 100_i64.into())
            .unwrap()
            .with_constant(14, 14_i32.into())
            .unwrap()
            .with_constant(18, base16::decode(MINERS_FEE_BASE16_BYTES).unwrap().into())
            .unwrap();
        deposit_box.ergo_tree = tree;

        println!("{:?}", deposit_box.box_id());
        let deposit = <AsBox<Deposit>>::try_from_box(deposit_box).unwrap();

        let ec = ExecutionContext {
            height: 921700,
            mintable_token_id: pool.0.box_id().into(),
            executor_prop: trivial_prop(),
        };

        let res = deposit.clone().try_run(pool, (), ec);

        println!("{:?}", res);
        assert!(res.is_ok());

        let prover = Wallet::trivial(Vec::new());

        let signed_tx = prover.sign(res.unwrap().0).unwrap();

        let client = HttpClient::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap();

        let node = ErgoNodeHttpClient::new(
            client,
            Url::try_from(String::from("http://213.239.193.208:9053")).unwrap(),
        );
        let res = node.submit_tx(signed_tx).await;
        println!("submit_tx: {:?}", res);
        assert!(res.is_ok());
    }

    #[test]
    fn redeemer_prop_roundtrip() {
        let sample = "0008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec";
        let tree = ErgoTree::sigma_parse_bytes(&base16::decode(sample).unwrap()).unwrap();
        let sigma_prop = SigmaProp::from(ProveDlog::try_from(tree).unwrap());
        let tree_reconstructed = ErgoTree::new(ErgoTreeHeader::v0(false), &sigma_prop.into()).unwrap();
        let tree_encoded = base16::encode_lower(&*tree_reconstructed.sigma_serialize_bytes().unwrap());

        assert_eq!(tree_encoded, sample);
    }

    const POOL_JSON: &str = r#"{
        "boxId": "7153d14ec1fad42943102cc0541c96e44549f5c4e496e27f43b0885fd4a9fd43",
        "value": 1250000,
        "ergoTree": "19ec052404000400040204020404040404060406040804080404040204000400040204020400040a050005000404040204020e2074aeba0675c10c7fff46d3aa5e5a8efc55f0b0d87393dcb2f4b0a04be213cecb040004020500040204020406050005000402050205000500d81ed601b2a5730000d602db63087201d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b27203730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb27202730800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d6148c720902d615b27205730a00d6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205730f00d61b7e721a05d61c9d7206721bd61d998c720c028c720d02d61e998c720f028c721002d1ededededed93b272027310007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a70193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b172027311959172137312d802d61f9c721399721ba273137e721905d620b2a5731400ededed929a997206721472079c7e9995907219721a72199a721a7315731605721c937213f0721d93721ff0721eedededed93cbc272207317938602720e7213b2db630872207318009386027211721fb2db63087220731900e6c67220040893e4c67220050e8c720401958f7213731aededec929a997206721472079c7e9995907219721a72199a721a731b731c05721c92a39a9a72159c721a7217b27205731d0093721df0721392721e95917219721a731e9c721d99721ba2731f7e721905d801d61fe4c672010704edededed90721f9972197320909972149c7e99721a721f05721c9a721c7207907ef0998c7208027214069d9c7e721c067e721e067e997212732106937213732293721d7323",
        "assets": [
            {
                "tokenId": "48ad28d9bb55e1da36d27c655a84279ff25d889063255d3f774ff926a3704370",
                "amount": 1
            },
            {
                "tokenId": "0779ec04f2fae64e87418a1ad917639d4668f78484f45df962b0dec14a2591d2",
                "amount": 300000
            },
            {
                "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                "amount": 1
            },
            {
                "tokenId": "3fdce3da8d364f13bca60998c20660c79c19923f44e141df01349d2e63651e86",
                "amount": 100000000
            },
            {
                "tokenId": "c256908dd9fd477bde350be6a41c0884713a1b1d589357ae731763455ef28c10",
                "amount": 1500000000
            }
        ],
        "creationHeight": 921585,
        "additionalRegisters": {
            "R4": "100490031eaac170c801",
            "R5": "05becf24",
            "R6": "05d00f"
        },
        "transactionId": "ea34ff4653ce1579cb96464014ca072d1574fc04ac58f159786c3a1debebac2b",
        "index": 0
    }"#;
}
