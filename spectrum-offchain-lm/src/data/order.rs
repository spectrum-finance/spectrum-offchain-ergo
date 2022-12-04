use spectrum_offchain::domain::TypedAssetAmount;

use crate::data::assets::{BundleKey, LQ};
use crate::data::{OrderId, PoolId};

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Deposit {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub lq: TypedAssetAmount<LQ>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Redeem {
    pub order_id: OrderId,
    pub pool_id: PoolId,
    pub bundle_key: TypedAssetAmount<BundleKey>,
    pub expected_lq: TypedAssetAmount<LQ>,
}
