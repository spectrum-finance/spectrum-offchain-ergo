use std::cmp::Ordering;
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use derive_more::{From, Into};
use ergo_lib::ergo_chain_types::Digest32;
use ergo_lib::ergotree_ir::chain::ergo_box::{BoxId, ErgoBox};
use ergo_lib::ergotree_ir::chain::token::TokenId;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use serde::ser::SerializeTupleStruct;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use type_equalities::IsEqual;

use spectrum_offchain::data::{Has, OnChainEntity, OnChainOrder};
use spectrum_offchain::event_sink::handlers::types::TryFromBox;

use crate::executor::{ConsumeExtra, ProduceExtra};

pub mod assets;
pub mod bundle;
pub mod context;
pub mod executor;
pub mod funding;
pub mod miner;
pub mod order;
pub mod pool;
pub mod redeemer;

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Serialize, Deserialize)]
pub struct FundingId(BoxId);

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Serialize, Deserialize)]
pub struct OrderId(Digest32);

impl Display for OrderId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&Digest32::from(self.0), f)
    }
}

impl From<BoxId> for OrderId {
    fn from(bx_id: BoxId) -> Self {
        OrderId(bx_id.into())
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Into, Serialize, Deserialize)]
#[serde(from = "PoolIdBytes")]
#[serde(into = "PoolIdBytes")]
pub struct PoolId(TokenId);

impl Display for PoolId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&Digest32::from(self.0), f)
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Into, Serialize, Deserialize)]
pub struct PoolIdBytes([u8; 32]);

impl From<PoolIdBytes> for PoolId {
    fn from(PoolIdBytes(xs): PoolIdBytes) -> Self {
        Self(TokenId::from(Digest32::from(xs)))
    }
}

impl From<PoolId> for PoolIdBytes {
    fn from(pid: PoolId) -> Self {
        Self(Digest32::from(TokenId::from(pid)).0)
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Into, Serialize, Deserialize)]
#[serde(from = "PoolStateIdBytes")]
#[serde(into = "PoolStateIdBytes")]
pub struct PoolStateId(BoxId);

impl Display for PoolStateId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&Digest32::from(self.0), f)
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Into, Serialize, Deserialize)]
pub struct PoolStateIdBytes([u8; 32]);

impl From<PoolStateId> for PoolStateIdBytes {
    fn from(sid: PoolStateId) -> Self {
        Self(Digest32::from(sid.0).0)
    }
}

impl From<PoolStateIdBytes> for PoolStateId {
    fn from(sid: PoolStateIdBytes) -> Self {
        Self(BoxId::from(Digest32::from(sid.0)))
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Into, Serialize, Deserialize)]
#[serde(from = "BundleIdBytes")]
#[serde(into = "BundleIdBytes")]
pub struct BundleId(TokenId);

impl PartialOrd for BundleId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        <Vec<u8>>::from(self.0).partial_cmp(&<Vec<u8>>::from(other.0))
    }
}

impl Ord for BundleId {
    fn cmp(&self, other: &Self) -> Ordering {
        <Vec<u8>>::from(self.0).cmp(&<Vec<u8>>::from(other.0))
    }
}

impl Display for BundleId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&Digest32::from(self.0), f)
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Into, Serialize, Deserialize)]
pub struct BundleIdBytes([u8; 32]);

impl From<BundleIdBytes> for BundleId {
    fn from(BundleIdBytes(xs): BundleIdBytes) -> Self {
        Self(TokenId::from(Digest32::from(xs)))
    }
}

impl From<BundleId> for BundleIdBytes {
    fn from(bid: BundleId) -> Self {
        Self(Digest32::from(TokenId::from(bid)).0)
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash, From, Serialize, Deserialize)]
pub struct BundleStateId(BoxId);

impl Display for BundleStateId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&Digest32::from(self.0), f)
    }
}

/// Something that is represented as an `ErgoBox` on-chain.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct AsBox<T>(pub ErgoBox, pub T);

impl<T> Hash for AsBox<T>
where
    T: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(&*self.0.sigma_serialize_bytes().unwrap());
        self.1.hash(state);
    }
}

impl<T> Serialize for AsBox<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut serde_state = match serializer.serialize_tuple_struct("AsBox", 2usize) {
            Ok(val) => val,
            Err(err) => {
                return Err(err);
            }
        };
        match serde_state.serialize_field(&self.0.sigma_serialize_bytes().unwrap()) {
            Ok(val) => val,
            Err(err) => {
                return Err(err);
            }
        };
        match serde_state.serialize_field(&self.1) {
            Ok(val) => val,
            Err(err) => {
                return Err(err);
            }
        };
        serde_state.end()
    }
}

impl<'de, T> Deserialize<'de> for AsBox<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor<'de, T>
        where
            T: Deserialize<'de>,
        {
            marker: PhantomData<AsBox<T>>,
            lifetime: PhantomData<&'de ()>,
        }
        impl<'de, T> de::Visitor<'de> for Visitor<'de, T>
        where
            T: Deserialize<'de>,
        {
            type Value = AsBox<T>;
            fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
                formatter.write_str("tuple struct AsBox")
            }
            #[inline]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let field0 = match match seq.next_element::<Vec<u8>>() {
                    Ok(val) => val,
                    Err(err) => {
                        return Err(err);
                    }
                } {
                    Some(value) => value,
                    None => {
                        return Err(de::Error::invalid_length(
                            0usize,
                            &"tuple struct AsBox with 2 elements",
                        ));
                    }
                };
                let field1 = match match seq.next_element() {
                    Ok(val) => val,
                    Err(err) => {
                        return Err(err);
                    }
                } {
                    Some(value) => value,
                    None => {
                        return Err(de::Error::invalid_length(
                            1usize,
                            &"tuple struct AsBox with 2 elements",
                        ));
                    }
                };
                Ok(AsBox(ErgoBox::sigma_parse_bytes(&*field0).unwrap(), field1))
            }
        }
        deserializer.deserialize_tuple_struct(
            "AsBox",
            2usize,
            Visitor {
                marker: PhantomData::<AsBox<T>>,
                lifetime: PhantomData,
            },
        )
    }
}

impl<T> ConsumeExtra for AsBox<T>
where
    T: ConsumeExtra,
{
    type TExtraIn = T::TExtraIn;
}

impl<T> ProduceExtra for AsBox<T>
where
    T: ProduceExtra,
{
    type TExtraOut = T::TExtraOut;
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

#[cfg(test)]
mod tests {
    use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
    use sigma_test_util::force_any_val;

    use crate::data::AsBox;

    #[test]
    fn as_box_serialize_deserialize() {
        let as_box = AsBox(force_any_val::<ErgoBox>(), 0u8);
        let bytes = bincode::serialize(&as_box).unwrap();
        let result: AsBox<u8> = bincode::deserialize(&bytes).unwrap();
        assert_eq!(as_box, result)
    }
}
