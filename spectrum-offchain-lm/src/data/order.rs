use std::hash::{Hash, Hasher};
use std::iter;

use ergo_lib::chain::transaction::TxIoVec;
use ergo_lib::ergo_chain_types::{blake2b256_hash, Digest32};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ContextExtension;
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
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
use crate::ergo::NanoErg;
use crate::executor::{ConsumeExtra, ProduceExtra, RunOrder};
use crate::token_details::TokenDetails;
use crate::validators::{BUNDLE_VALIDATOR, DEPOSIT_TEMPLATE, REDEEM_TEMPLATE};

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub struct Compound {
    pub pool_id: PoolId,
    pub epoch_ix: u32,
    pub queue_ix: usize,
    pub stakers: Vec<BundleId>,
}

impl Compound {
    pub fn order_id(&self) -> OrderId {
        OrderId::from(blake2b256_hash(
            &self
                .stakers
                .iter()
                .map(|bid| <Vec<u8>>::from(bid.0))
                .chain(iter::once(self.epoch_ix.to_be_bytes().to_vec()))
                .flatten()
                .collect::<Vec<u8>>(),
        ))
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
                                .map(|nf| vec![nf.into_candidate(ctx.height)])
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
                let num_bundles = bundles.len();
                let (first_bundle_out_ix, _) = outputs
                    .iter()
                    .find_position(|o| o.ergo_tree == *BUNDLE_VALIDATOR)
                    .expect("Bundles must always be present in outputs");
                let inputs = TxIoVec::from_vec(
                    vec![pool_in]
                        .into_iter()
                        .chain(funding.map(|AsBox(i, _)| i))
                        .map(|bx| (bx, ContextExtension::empty()))
                        .chain(bundles.iter().enumerate().map(|(i, AsBox(bx, _))| {
                            let succ_ix = first_bundle_out_ix + i;
                            let redeemer_out_ix = succ_ix + num_bundles;
                            let mut constants = IndexMap::new();
                            constants.insert(0u8, Constant::from(redeemer_out_ix as i32));
                            constants.insert(1u8, Constant::from(succ_ix as i32));
                            (bx.clone(), ContextExtension { values: constants })
                        }))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let tx = TransactionCandidate::new(inputs, None, outputs);
                let outputs = tx.clone().into_tx_without_proofs().outputs;
                let next_pool_as_box = AsBox(outputs.get(0).unwrap().clone(), next_pool);
                let bun_init_ix = if next_funding.is_some() { 2 } else { 1 };
                let bundle_outs = &outputs[bun_init_ix..bun_init_ix + next_bundles.len()];
                let bundles_as_box = next_bundles
                    .into_iter()
                    .zip(Vec::from(bundle_outs).into_iter())
                    .map(|(mut bn, out)| {
                        bn.state_id = BundleStateId::from(out.box_id());
                        Predicted(AsBox(out, bn))
                    })
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
        state.write(&self.redeemer_prop.prop_bytes().unwrap());
        self.bundle_prop_hash.hash(state);
        self.max_miner_fee.hash(state);
        self.lq.hash(state);
        self.erg_value.hash(state);
        self.expected_num_epochs.hash(state);
    }
}

impl ConsumeExtra for Deposit {
    type TExtraIn = TokenDetails;
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
        token_details: TokenDetails,
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
        match pool.apply_deposit(self_order, token_details, ctx.clone()) {
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
                                .get_constant(2)
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
                    .get_constant(12)
                    .ok()??
                    .v
                    .try_extract_into::<Digest32>()
                    .ok()?;
                let max_miner_fee = bx
                    .ergo_tree
                    .get_constant(23)
                    .ok()??
                    .v
                    .try_extract_into::<i64>()
                    .ok()?;
                let expected_num_epochs = bx
                    .ergo_tree
                    .get_constant(16)
                    .ok()??
                    .v
                    .try_extract_into::<i32>()
                    .ok()?;
                let miner_prop_bytes = bx
                    .ergo_tree
                    .get_constant(20)
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

/// Contains all information that can be extracted from a `Redeem` box. It's missing the pool Id
/// since that can only be obtained by inspecting an associated staking bundle.
#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
#[serde(from = "RawRedeemProto")]
#[serde(into = "RawRedeemProto")]
pub struct RedeemProto {
    pub order_id: OrderId,
    pub redeemer_prop: ErgoTree,
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub expected_lq: TypedAssetAmount<Lq>,
    pub max_miner_fee: i64,
    pub erg_value: NanoErg,
}

impl RedeemProto {
    pub fn finalize(self, pool_id: PoolId) -> Redeem {
        Redeem {
            order_id: self.order_id,
            pool_id,
            redeemer_prop: self.redeemer_prop,
            bundle_key: self.bundle_key,
            expected_lq: self.expected_lq,
            max_miner_fee: self.max_miner_fee,
            erg_value: self.erg_value,
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

impl From<RawRedeem> for Redeem {
    fn from(rr: RawRedeem) -> Self {
        Self {
            order_id: rr.order_id,
            pool_id: rr.pool_id,
            redeemer_prop: ErgoTree::sigma_parse_bytes(&rr.redeemer_prop_bytes).unwrap(),
            bundle_key: TypedAssetAmount::new(rr.bundle_key.0, rr.bundle_key.1),
            expected_lq: TypedAssetAmount::new(rr.expected_lq.0, rr.expected_lq.1),
            max_miner_fee: rr.max_miner_fee,
            erg_value: NanoErg::from(rr.erg_value),
        }
    }
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

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct RawRedeemProto {
    pub order_id: OrderId,
    pub redeemer_prop_bytes: Vec<u8>,
    pub bundle_key: (TokenId, u64),
    pub expected_lq: (TokenId, u64),
    pub max_miner_fee: i64,
    pub erg_value: u64,
}

impl From<RedeemProto> for RawRedeemProto {
    fn from(r: RedeemProto) -> Self {
        Self {
            order_id: r.order_id,
            redeemer_prop_bytes: r.redeemer_prop.sigma_serialize_bytes().unwrap(),
            bundle_key: (r.bundle_key.token_id, r.bundle_key.amount),
            expected_lq: (r.expected_lq.token_id, r.expected_lq.amount),
            max_miner_fee: r.max_miner_fee,
            erg_value: r.erg_value.into(),
        }
    }
}

impl From<RawRedeemProto> for RedeemProto {
    fn from(rr: RawRedeemProto) -> Self {
        Self {
            order_id: rr.order_id,
            redeemer_prop: ErgoTree::sigma_parse_bytes(&rr.redeemer_prop_bytes).unwrap(),
            bundle_key: TypedAssetAmount::new(rr.bundle_key.0, rr.bundle_key.1),
            expected_lq: TypedAssetAmount::new(rr.expected_lq.0, rr.expected_lq.1),
            max_miner_fee: rr.max_miner_fee,
            erg_value: NanoErg::from(rr.erg_value),
        }
    }
}

impl Hash for Redeem {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.order_id.hash(state);
        state.write(&self.redeemer_prop.sigma_serialize_bytes().unwrap());
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

impl Has<OrderId> for RedeemProto {
    fn get<U: IsEqual<OrderId>>(&self) -> OrderId {
        self.order_id
    }
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

impl TryFromBox for RedeemProto {
    fn try_from_box(bx: ErgoBox) -> Option<RedeemProto> {
        if let Some(ref tokens) = bx.tokens {
            if bx.ergo_tree.template_bytes().ok()? == *REDEEM_TEMPLATE && tokens.len() == 1 {
                let order_id = OrderId::from(bx.box_id());
                let redeemer_prop = ErgoTree::sigma_parse_bytes(
                    &*bx.ergo_tree
                        .get_constant(9)
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
                            .get_constant(10)
                            .ok()??
                            .v
                            .try_extract_into::<Vec<u8>>()
                            .ok()?,
                    )
                    .ok()?,
                );
                let expected_lq_amt = bx
                    .ergo_tree
                    .get_constant(11)
                    .ok()??
                    .v
                    .try_extract_into::<i64>()
                    .ok()? as u64;
                let expected_lq = TypedAssetAmount::new(expected_lq_id, expected_lq_amt);
                let miner_prop_bytes = bx
                    .ergo_tree
                    .get_constant(5)
                    .ok()??
                    .v
                    .try_extract_into::<Vec<u8>>()
                    .ok()?;
                assert_eq!(miner_prop_bytes, base16::decode(MINERS_FEE_BASE16_BYTES).unwrap());
                let max_miner_fee = bx
                    .ergo_tree
                    .get_constant(8)
                    .ok()??
                    .v
                    .try_extract_into::<i64>()
                    .ok()?;
                return Some(RedeemProto {
                    order_id,
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

/// This type exists to wrap over `RedeemProto`.
#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub enum OrderProto {
    Deposit(AsBox<Deposit>),
    Redeem(AsBox<RedeemProto>),
    Compound(Compound),
}

impl Has<OrderId> for OrderProto {
    fn get<U: IsEqual<OrderId>>(&self) -> OrderId {
        match self {
            OrderProto::Deposit(AsBox(_, deposit)) => deposit.order_id,
            OrderProto::Redeem(AsBox(_, redeem)) => redeem.order_id,
            OrderProto::Compound(compound) => compound.order_id(),
        }
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

impl TryFromBox for OrderProto {
    fn try_from_box(bx: ErgoBox) -> Option<OrderProto> {
        Deposit::try_from_box(bx.clone())
            .map(|d| OrderProto::Deposit(AsBox(bx.clone(), d)))
            .or_else(|| RedeemProto::try_from_box(bx.clone()).map(|r| OrderProto::Redeem(AsBox(bx, r))))
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
    use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
    use ergo_lib::ergotree_ir::ergo_tree::{ErgoTree, ErgoTreeHeader};
    use ergo_lib::ergotree_ir::mir::constant::Constant;
    use ergo_lib::ergotree_ir::mir::expr::Expr;
    use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
    use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::{ProveDlog, SigmaProp};

    use spectrum_offchain::event_sink::handlers::types::TryFromBox;

    use crate::data::context::ExecutionContext;
    use crate::data::order::{Deposit, OrderProto};
    use crate::data::pool::Pool;
    use crate::data::AsBox;
    use crate::executor::RunOrder;
    use crate::prover::{SigmaProver, Wallet};
    use crate::token_details::TokenDetails;

    use super::RedeemProto;

    fn trivial_prop() -> ErgoTree {
        ErgoTree::try_from(Expr::Const(Constant::from(true))).unwrap()
    }

    #[test]
    fn parse_order() {
        let sample_json = r#"{
            "boxId": "bb509e85cdfe71a15c57396552c830259cfcb8020c1d637abc0899de08fa53b0",
            "value": 2750000,
            "ergoTree": "19a6041904000e203130a82e45842aebb888742868e055e2f554ab7d92f233f2c828ed4a437937100e240008cd03df90a38ed9b551bd7f706f251ae84974179e44e1d2113794dc115b769672fe3f08cd02217daf90deb73bdf8b6709bb42093fdfaff6573fd47b630e2d3fdd4a8193a74d0404040a040204040400040005fcffffffffffffffff0104000e20156dd1feb11debf65765c1d28fcf2e599a280789ee60ad426ccdeceb0ce22541040604000408041c0402050204040e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a573040500050005c0cf240100d803d601b2a4730000d6027301d6037302eb027303d195ed92b1a4730493b1db630872017305d805d604db63087201d605b2a5730600d606c57201d607b2a5730700d6088cb2db6308a773080002ededed938cb27204730900017202ed93c2720572039386027206730ab2db63087205730b00ededededed93cbc27207730c93d0e4c672070608720393e4c67207070e72029386028cb27204730d00017208b2db63087207730e009386028cb27204730f00019c72087e731005b2db6308720773110093860272067312b2db6308720773130090b0ada5d90109639593c272097314c1720973157316d90109599a8c7209018c72090273177318",
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
        let res = OrderProto::try_from_box(bx);
        println!("{:?}", res);
        assert!(res.is_some())
    }

    /// Used to generate a serialised ergotree for testing
    fn gen_deposit_ergotree() {
        let base16_str = "19a2041904000e2002020202020202020202020202020202020202020202020202020202020202020e20000000000000000000000000000000000000000000000000000000000000000008cd02217daf90deb73bdf8b6709bb42093fdfaff6573fd47b630e2d3fdd4a8193a74d0404040a040204040400040005fcffffffffffffffff0104000e200508f3623d4b2be3bdb9737b3e65644f011167eefb830d9965205f022ceda40d04060400040804140402050204040e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a573040500050005a09c010100d803d601b2a4730000d6027301d6037302eb027303d195ed92b1a4730493b1db630872017305d805d604db63087201d605b2a5730600d606c57201d607b2a5730700d6088cb2db6308a773080002ededed938cb27204730900017202ed93c2720572039386027206730ab2db63087205730b00ededededed93cbc27207730c93d0e4c672070608720393e4c67207070e72029386028cb27204730d00017208b2db63087207730e009386028cb27204730f00019c72087e731005b2db6308720773110093860272067312b2db6308720773130090b0ada5d90109639593c272097314c1720973157316d90109599a8c7209018c72090273177318";
        let tree_bytes = base16::decode(base16_str.as_bytes()).unwrap();

        let prover_input = force_any_val::<DlogProverInput>();
        let addr = Address::P2Pk(prover_input.public_image());
        let guard = addr.script().unwrap();
        let redeemer_prop = SigmaProp::from(ProveDlog::try_from(guard).unwrap())
            .prop_bytes()
            .unwrap();

        let pool_id = force_any_val::<TokenId>();
        let bundle_prop_hash = force_any_val::<Digest32>();
        let max_miner_fee = 300000_i64;
        let expected_num_epochs = 14_i32; // MUST BE 14 to be consistent with the pool box used in tests.
        let miner_prop_bytes = base16::decode(MINERS_FEE_BASE16_BYTES).unwrap();

        let tree = ErgoTree::sigma_parse_bytes(&tree_bytes)
            .unwrap()
            .with_constant(1, pool_id.into())
            .unwrap()
            .with_constant(2, redeemer_prop.into())
            .unwrap()
            .with_constant(12, bundle_prop_hash.into())
            .unwrap()
            .with_constant(23, max_miner_fee.into())
            .unwrap()
            .with_constant(16, expected_num_epochs.into())
            .unwrap()
            .with_constant(20, miner_prop_bytes.into())
            .unwrap();

        println!("ERGOTREE BYTES: {}", tree.to_base16_bytes().unwrap());
    }

    #[test]
    fn run_deposit() {
        let deposit_json = r#"{
            "boxId": "bb509e85cdfe71a15c57396552c830259cfcb8020c1d637abc0899de08fa53b0",
            "value": 2750000,
            "ergoTree": "19a6041904000e203130a82e45842aebb888742868e055e2f554ab7d92f233f2c828ed4a437937100e240008cd03df90a38ed9b551bd7f706f251ae84974179e44e1d2113794dc115b769672fe3f08cd02217daf90deb73bdf8b6709bb42093fdfaff6573fd47b630e2d3fdd4a8193a74d0404040a040204040400040005fcffffffffffffffff0104000e20156dd1feb11debf65765c1d28fcf2e599a280789ee60ad426ccdeceb0ce22541040604000408041c0402050204040e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a573040500050005c0cf240100d803d601b2a4730000d6027301d6037302eb027303d195ed92b1a4730493b1db630872017305d805d604db63087201d605b2a5730600d606c57201d607b2a5730700d6088cb2db6308a773080002ededed938cb27204730900017202ed93c2720572039386027206730ab2db63087205730b00ededededed93cbc27207730c93d0e4c672070608720393e4c67207070e72029386028cb27204730d00017208b2db63087207730e009386028cb27204730f00019c72087e731005b2db6308720773110093860272067312b2db6308720773130090b0ada5d90109639593c272097314c1720973157316d90109599a8c7209018c72090273177318",
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

        let token_details = TokenDetails {
            name: String::from(""),
            description: String::from(""),
        };
        let res = deposit.try_run(pool, token_details, ec);

        println!("{:?}", res);
        assert!(res.is_ok());

        let prover = Wallet::trivial(Vec::new());

        //println!("{}", serde_json::to_string(&res.unwrap().0.into_tx_without_proofs()).unwrap());

        let signed_tx = prover.sign(res.unwrap().0);

        assert!(signed_tx.is_ok());
    }

    #[test]
    fn test_redeem_from_box() {
        let redeem_json = r#"
        {
            "boxId" : "f711dbb1ddecacc0c23ccc5806dcb39890d30634e9b99339cc11aca42be721d4",
            "value" : 2500000,
            "ergoTree" : "19d1020e08cd02217daf90deb73bdf8b6709bb42093fdfaff6573fd47b630e2d3fdd4a8193a74d04040400040a04020e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a573040500050005a09c010e2001010101010101010101010101010101010101010101010101010101010101010e20000000000000000000000000000000000000000000000000000000000000000005d00f04000100eb027300d195ed92b1a4730193b1db6308b2a47302007303d802d601b2a5730400d60290b0ada5d90102639593c272027305c1720273067307d90102599a8c7202018c7202027308ededed93c272017309938602730a730bb2db63087201730c0072027202730d",
            "assets" : [
              {
                "tokenId" : "8830d8d6f5501156bfd1e1a59e9399199f7c4afb941899c685fe809da23fd954",
                "amount" : 9223372036854775806
              }
            ],
            "creationHeight" : 944473,
            "additionalRegisters" : {
              
            },
            "transactionId" : "014b195d069ce7510335a28b8ab51d98847829bf58da8aa14ccf54526853e149",
            "index" : 0
          }
        "#;
        let redeem_box: ErgoBox = serde_json::from_str(redeem_json).unwrap();
        let _ = RedeemProto::try_from_box(redeem_box).unwrap();
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
        "boxId": "6eeb78dacf40d75ea9421206ab6ff71df9e67b80212d47766ea3c648957d7802",
        "value": 1250000,
        "ergoTree": "19c0062804000400040204020404040404060406040804080404040204000400040204020400040a050005000404040204020e200508f3623d4b2be3bdb9737b3e65644f011167eefb830d9965205f022ceda40d0400040205000402040204060500050005feffffffffffffffff01050005000402060101050005000100d81fd601b2a5730000d602db63087201d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b27203730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb27202730800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d6148c720902d615b27205730a00d6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205730f00d61b7e721a05d61c9d7206721bd61d998c720c028c720d02d61e8c721002d61f998c720f02721ed1ededededed93b272027310007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a70193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b172027311959172137312d802d6209c721399721ba273137e721905d621b2a5731400ededed929a7e9972067214067e7207067e9c7e9995907219721a72199a721a7315731605721c06937213f0721d937220f0721fedededed93cbc272217317938602720e7213b2db6308722173180093860272117220b2db63087221731900e6c67221060893e4c67221070e8c720401958f7213731aededec929a7e9972067214067e7207067e9c7e9995907219721a72199a721a731b731c05721c0692a39a9a72159c721a7217b27205731d0093721df0721392721f95917219721a731e9c721d99721ba2731f7e721905d804d620e4c672010704d62199721a7220d6227e722105d62399997320721e9c7212722295ed917223732191721f7322edededed9072209972197323909972149c7222721c9a721c7207907ef0998c7208027214069d9c99997e7214069d9c7e7206067e7221067e721a0673247e721f067e722306937213732593721d73267327",
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
