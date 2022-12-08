use std::fmt::{Debug, Display, Formatter};

use derive_more::{From, Into};
use ergo_lib::ergo_chain_types::Digest32;
use ergo_lib::ergotree_ir::chain::ergo_box::{BoxId, ErgoBox};
use ergo_lib::ergotree_ir::chain::token::TokenId;
use type_equalities::IsEqual;

use spectrum_offchain::data::{Has, OnChainEntity, OnChainOrder};
use spectrum_offchain::event_sink::handlers::types::TryFromBox;

use crate::executor::{ConsumeBundle, ProduceBundle};

pub mod assets;
pub mod bundle;
pub mod context;
pub mod order;
pub mod pool;
pub mod redeemer;

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From)]
pub struct OrderId(Digest32);

impl From<BoxId> for OrderId {
    fn from(bx_id: BoxId) -> Self {
        OrderId(bx_id.into())
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Into)]
pub struct PoolId(TokenId);

impl Display for PoolId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&Digest32::from(self.0), f)
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From)]
pub struct PoolStateId(BoxId);

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From)]
pub struct BundleId(TokenId);

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From)]
pub struct BundleStateId(BoxId);

/// Something that is represented as an `ErgoBox` on-chain.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct AsBox<T>(pub ErgoBox, pub T);

impl<T> ConsumeBundle for AsBox<T>
where
    T: ConsumeBundle,
{
    type TBundleIn = T::TBundleIn;
}

impl<T> ProduceBundle for AsBox<T>
where
    T: ProduceBundle,
{
    type TBundleOut = T::TBundleOut;
}

impl<T> AsBox<T> {
    pub fn box_id(&self) -> BoxId {
        self.0.box_id()
    }

    pub fn map<F, U>(self, f: F) -> AsBox<U>
    where
        F: FnOnce(T) -> U,
    {
        let AsBox(bx, t) = self;
        AsBox(bx, f(t))
    }
}

impl<T, K> Has<K> for AsBox<T>
where
    T: Has<K>,
{
    fn get<U: IsEqual<K>>(&self) -> K {
        self.1.get::<K>()
    }
}

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
