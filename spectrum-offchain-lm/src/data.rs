use derive_more::From;
use ergo_lib::ergotree_ir::chain::ergo_box::{BoxId, ErgoBox};
use ergo_lib::ergotree_ir::chain::token::TokenId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::data::{OnChainEntity, OnChainOrder};
use spectrum_offchain::domain::TypedAsset;
use spectrum_offchain::event_sink::handlers::types::TryFromBox;

use crate::data::assets::{BundleKey, PoolNft};

pub mod assets;
pub mod bundle;
pub mod order;
pub mod pool;
pub mod redeemer;

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From)]
pub struct OrderId(BoxId);

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From)]
pub struct PoolId(TokenId);

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From)]
pub struct PoolStateId(BoxId);

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From)]
pub struct BundleId(TokenId);

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From)]
pub struct BundleStateId(BoxId);

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct LmContext {
    pub executor_prop: ErgoTree,
}

/// Something that is represented with an `ErgoBox` on-chain.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct AsBox<T>(ErgoBox, T);

impl<T> TryFromBox for AsBox<T>
where
    T: TryFromBox,
{
    fn try_from_box(bx: ErgoBox) -> Option<AsBox<T>> {
        T::try_from_box(bx.clone()).map(|x| AsBox(bx, x))
    }
}

impl<T> OnChainEntity for AsBox<T>
where
    T: OnChainEntity,
{
    type TEntityId = T::TEntityId;
    type TStateId = T::TStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.1.get_self_ref()
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.1.get_self_state_ref()
    }
}

impl<T> OnChainOrder for AsBox<T>
where
    T: OnChainOrder,
{
    type TOrderId = T::TOrderId;
    type TEntityId = T::TEntityId;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.1.get_self_ref()
    }

    fn get_entity_ref(&self) -> Self::TEntityId {
        self.1.get_entity_ref()
    }
}
