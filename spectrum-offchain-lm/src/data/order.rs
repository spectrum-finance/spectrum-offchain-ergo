use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::ProveDlog;
use type_equalities::IsEqual;

use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Has, OnChainOrder};
use spectrum_offchain::domain::TypedAssetAmount;
use spectrum_offchain::event_sink::handlers::types::TryFromBox;
use spectrum_offchain::executor::RunOrderFailure;

use crate::data::assets::{BundleKey, Lq};
use crate::data::bundle::StakingBundle;
use crate::data::pool::Pool;
use crate::data::{BundleId, LmContext, OrderId, PoolId};
use crate::executor::RunOrder;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Deposit {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub redeemer_prop: ErgoTree,
    pub refund_pk: ProveDlog,
    pub lq: TypedAssetAmount<Lq>,
}

pub struct DepositParser {
    deposit_validator_template: Vec<u8>,
}

impl TryFromBox<Deposit> for DepositParser {
    fn try_from(&self, bx: ErgoBox) -> Option<Deposit> {
        todo!()
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Redeem {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub redeemer_prop: ErgoTree,
    pub refund_pk: ProveDlog,
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub expected_lq: TypedAssetAmount<Lq>,
}

pub struct RedeemParser {
    redeem_validator_template: Vec<u8>,
}

impl TryFromBox<Redeem> for RedeemParser {
    fn try_from(&self, bx: ErgoBox) -> Option<Redeem> {
        todo!()
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Order {
    Deposit(Deposit),
    Redeem(Redeem),
}

impl OnChainOrder for Order {
    type TOrderId = OrderId;
    type TEntityId = PoolId;

    fn get_self_ref(&self) -> Self::TOrderId {
        match self {
            Order::Deposit(deposit) => deposit.order_id.clone(), // todo: remove .clone() when sigma is updated.
            Order::Redeem(redeem) => redeem.order_id.clone(), // todo: remove .clone() when sigma is updated.
        }
    }

    fn get_entity_ref(&self) -> Self::TEntityId {
        match self {
            Order::Deposit(deposit) => deposit.pool_id.clone(), // todo: remove .clone() when sigma is updated.
            Order::Redeem(redeem) => redeem.pool_id.clone(), // todo: remove .clone() when sigma is updated.
        }
    }
}

impl Has<Option<BundleId>> for Order {
    fn get<U: IsEqual<Option<BundleId>>>(&self) -> Option<BundleId> {
        match self {
            Order::Deposit(_) => None,
            Order::Redeem(redeem) => Some(BundleId::from(redeem.bundle_key.clone().to_asset())), // todo: remove .clone() when sigma is updated.
        }
    }
}

impl RunOrder for Order {
    fn try_run(
        self,
        pool: Pool,
        bundle: Option<StakingBundle>,
        ctx: LmContext,
    ) -> Result<(Transaction, Predicted<Pool>, Option<Predicted<StakingBundle>>), RunOrderFailure<Self>> {
        todo!()
    }
}

pub struct OrderParser {}

impl TryFromBox<Order> for OrderParser {
    fn try_from(&self, bx: ErgoBox) -> Option<Order> {
        todo!()
    }
}
