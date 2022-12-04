use derive_more::From;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::domain::TypedAsset;

use crate::data::assets::{BundleKey, PoolNft};
use crate::data::bundle::StakingBundle;

pub mod assets;
pub mod bundle;
pub mod order;
pub mod pool;

#[derive(Debug, Eq, PartialEq, Clone, Hash, From)]
pub struct OrderId(BoxId);

#[derive(Debug, Eq, PartialEq, Clone, Hash, From)]
pub struct PoolId(TypedAsset<PoolNft>);

#[derive(Debug, Eq, PartialEq, Clone, Hash, From)]
pub struct PoolStateId(BoxId);

#[derive(Debug, Eq, PartialEq, Clone, Hash, From)]
pub struct BundleId(TypedAsset<BundleKey>);

#[derive(Debug, Eq, PartialEq, Clone, Hash, From)]
pub struct BundleStateId(BoxId);

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct LmContext {
    pub executor_prop: ErgoTree,
}
