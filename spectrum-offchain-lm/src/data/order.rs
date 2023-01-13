use std::hash::{Hash, Hasher};

use ergo_lib::chain::transaction::TxIoVec;
use ergo_lib::ergo_chain_types::{blake2b256_hash, Digest32};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ContextExtension;
use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, NonMandatoryRegisterId};
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::token::TokenId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::{Constant, TryExtractInto};
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use indexmap::IndexMap;
use itertools::Itertools;
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use type_equalities::IsEqual;

use spectrum_offchain::backlog::data::{OrderWeight, Weighted};
use spectrum_offchain::data::{Has, OnChainOrder};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::domain::TypedAssetAmount;
use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox};
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::transaction::{TransactionCandidate, UnsignedTransactionOps};

use crate::data::{AsBox, BundleId, BundleStateId, FundingId, OrderId, PoolId};
use crate::data::assets::{BundleKey, Lq};
use crate::data::bundle::StakingBundle;
use crate::data::context::ExecutionContext;
use crate::data::funding::DistributionFunding;
use crate::data::pool::{Pool, PoolOperationError};
use crate::ergo::NanoErg;
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
                                &*bx.get_register(NonMandatoryRegisterId::R4.into())
                                    .unwrap()
                                    .v
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
    pub redeemer_prop: ErgoTree,
    pub lq: TypedAssetAmount<Lq>,
    pub erg_value: NanoErg,
    pub expected_num_epochs: u32,
}

impl From<RawDeposit> for Deposit {
    fn from(rd: RawDeposit) -> Self {
        Self {
            order_id: rd.order_id,
            pool_id: rd.pool_id,
            redeemer_prop: ErgoTree::sigma_parse_bytes(&*rd.redeemer_prop_raw).unwrap(),
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
    pub lq: (TokenId, u64),
    pub erg_value: u64,
    pub expected_num_epochs: u32,
}

impl From<Deposit> for RawDeposit {
    fn from(d: Deposit) -> Self {
        Self {
            order_id: d.order_id,
            pool_id: d.pool_id,
            redeemer_prop_raw: d.redeemer_prop.sigma_serialize_bytes().unwrap(),
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
        state.write(&*self.redeemer_prop.sigma_serialize_bytes().unwrap());
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
                    .get_constant(14)
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

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
#[serde(from = "RawRedeem")]
#[serde(into = "RawRedeem")]
pub struct Redeem {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub redeemer_prop: ErgoTree,
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub expected_lq: TypedAssetAmount<Lq>,
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
            erg_value: r.erg_value.into(),
        }
    }
}

impl Hash for Redeem {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.order_id.hash(state);
        self.pool_id.hash(state);
        state.write(&*self.redeemer_prop.sigma_serialize_bytes().unwrap());
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
    use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
    use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
    use ergo_lib::ergotree_ir::mir::constant::Constant;
    use ergo_lib::ergotree_ir::mir::expr::Expr;

    use spectrum_offchain::event_sink::handlers::types::TryFromBox;
    use spectrum_offchain::executor::RunOrderError::Fatal;

    use crate::data::AsBox;
    use crate::data::context::ExecutionContext;
    use crate::data::order::{Deposit, Order};
    use crate::data::pool::PermanentError::LowValue;
    use crate::data::pool::Pool;
    use crate::executor::RunOrder;
    use crate::prover::{SigmaProver, Wallet, WalletSecret};

    fn trivial_prop() -> ErgoTree {
        ErgoTree::try_from(Expr::Const(Constant::from(true))).unwrap()
    }

    #[test]
    fn parse_order() {
        let sample_json = r#"{
            "boxId": "f95eccd8ec785ecd0a2ed5188d0ad7e05694b34f6c3b9ff44dc99586947d3a73",
            "value": 2750000,
            "ergoTree": "198c041604000e205c7b7988f34bfb13059c447c648c23c36d121228588fa9ea68b9943b0333ea4c04020e240008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec0404040008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec040005fcffffffffffffffff0104000e20599e30a83bc971f75582f2581f0633eebfe936b95d956ed103cbec520d80438604060400040804140402050204040e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a5730405000500058092f401d808d601b2a4730000d602db63087201d6037301d604b2a5730200d6057303d606c57201d607b2a5730400d6088cb2db6308a773050002eb027306d1ededed938cb27202730700017203ed93c27204720593860272067308b2db63087204730900ededededed93cbc27207730a93e4c67207040e720593e4c67207050e72039386028cb27202730b00017208b2db63087207730c009386028cb27202730d00019c72087e730e05b2db63087207730f0093860272067310b2db6308720773110090b0ada5d90109639593c272097312c1720973137314d90109599a8c7209018c7209027315",
            "assets": [
                {
                    "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                    "amount": 109
                }
            ],
            "creationHeight": 914528,
            "additionalRegisters": {},
            "transactionId": "dabcb750b045ba34e11b4e644ad6e27ba5a92d1c3b11b0278718d389d984e3a0",
            "index": 0
        }"#;
        let bx: ErgoBox = serde_json::from_str(sample_json).unwrap();
        let res = Order::try_from_box(bx);
        assert!(res.is_some())
    }

    #[test]
    fn run_deposit() {
        let deposit_json = r#"{
            "boxId": "f95eccd8ec785ecd0a2ed5188d0ad7e05694b34f6c3b9ff44dc99586947d3a73",
            "value": 2750000,
            "ergoTree": "198c041604000e205c7b7988f34bfb13059c447c648c23c36d121228588fa9ea68b9943b0333ea4c04020e240008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec0404040008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec040005fcffffffffffffffff0104000e20599e30a83bc971f75582f2581f0633eebfe936b95d956ed103cbec520d80438604060400040804140402050204040e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a5730405000500058092f401d808d601b2a4730000d602db63087201d6037301d604b2a5730200d6057303d606c57201d607b2a5730400d6088cb2db6308a773050002eb027306d1ededed938cb27202730700017203ed93c27204720593860272067308b2db63087204730900ededededed93cbc27207730a93e4c67207040e720593e4c67207050e72039386028cb27202730b00017208b2db63087207730c009386028cb27202730d00019c72087e730e05b2db63087207730f0093860272067310b2db6308720773110090b0ada5d90109639593c272097312c1720973137314d90109599a8c7209018c7209027315",
            "assets": [
                {
                    "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                    "amount": 109
                }
            ],
            "creationHeight": 914528,
            "additionalRegisters": {},
            "transactionId": "dabcb750b045ba34e11b4e644ad6e27ba5a92d1c3b11b0278718d389d984e3a0",
            "index": 0
        }"#;
        let pool_box: ErgoBox = serde_json::from_str(POOL_JSON).unwrap();
        let pool = <AsBox<Pool>>::try_from_box(pool_box).unwrap();
        let deposit_box: ErgoBox = serde_json::from_str(deposit_json).unwrap();
        let deposit = <AsBox<Deposit>>::try_from_box(deposit_box).unwrap();

        let ec = ExecutionContext {
            height: 914530,
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

    const POOL_JSON: &str = r#"{
        "boxId": "6a928dad5999caeaf08396f9a40c37b1b09ce2b972df457f4afacb61ff69009a",
        "value": 1250000,
        "ergoTree": "19e9041f04000402040204040404040604060408040804040402040004000402040204000400040a0500040204020500050004020402040605000500040205000500d81bd601b2a5730000d602db63087201d603db6308a7d604e4c6a70410d605e4c6a70505d606e4c6a70605d607b27202730100d608b27203730200d609b27202730300d60ab27203730400d60bb27202730500d60cb27203730600d60db27202730700d60eb27203730800d60f8c720a02d610998c720902720fd6118c720802d612b27204730900d6139a99a37212730ad614b27204730b00d6159d72137214d61695919e72137214730c9a7215730d7215d617b27204730e00d6187e721705d6199d72057218d61a998c720b028c720c02d61b998c720d028c720e02d1ededededed93b27202730f00b27203731000ededed93e4c672010410720493e4c672010505720593e4c6720106057206928cc77201018cc7a70193c27201c2a7ededed938c7207018c720801938c7209018c720a01938c720b018c720c01938c720d018c720e0193b172027311959172107312eded929a997205721172069c7e9995907216721772169a721773137314057219937210f0721a939c7210997218a273157e721605f0721b958f72107316ededec929a997205721172069c7e9995907216721772169a72177317731805721992a39a9a72129c72177214b2720473190093721af0721092721b959172167217731a9c721a997218a2731b7e721605d801d61ce4c672010704edededed90721c997216731c909972119c7e997217721c0572199a7219720693f0998c72070272117d9d9c7e7219067e721b067e720f0605937210731d93721a731e",
        "assets": [
            {
                "tokenId": "5c7b7988f34bfb13059c447c648c23c36d121228588fa9ea68b9943b0333ea4c",
                "amount": 1
            },
            {
                "tokenId": "0779ec04f2fae64e87418a1ad917639d4668f78484f45df962b0dec14a2591d2",
                "amount": 10000
            },
            {
                "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                "amount": 100
            },
            {
                "tokenId": "3fdce3da8d364f13bca60998c20660c79c19923f44e141df01349d2e63651e86",
                "amount": 10000000
            },
            {
                "tokenId": "c256908dd9fd477bde350be6a41c0884713a1b1d589357ae731763455ef28c10",
                "amount": 99999000
            }
        ],
        "creationHeight": 913825,
        "additionalRegisters": {
            "R4": "1004d00f1492d66fd00f",
            "R5": "05a09c01",
            "R6": "05d00f"
        },
        "transactionId": "ceb08428111f9d4dbcba3b0de024326af032e52f37ea189b8bd3affbcc40a3a6",
        "index": 0
    }"#;
}
