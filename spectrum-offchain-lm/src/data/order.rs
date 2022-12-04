use ergo_lib::chain::transaction::Transaction;
use type_equalities::IsEqual;

use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Has, OnChainOrder};
use spectrum_offchain::domain::TypedAssetAmount;
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
    pub lq: TypedAssetAmount<Lq>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Redeem {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub expected_lq: TypedAssetAmount<Lq>,
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
