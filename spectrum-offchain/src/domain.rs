use std::marker::PhantomData;
use std::ops::{Add, Sub};

use ergo_lib::ergotree_ir::chain::token::{Token, TokenId};

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash)]
pub struct TypedAsset<T> {
    pub token_id: TokenId,
    pub pd: PhantomData<T>,
}

impl<T> TypedAsset<T> {
    pub fn new(token_id: TokenId) -> Self {
        Self {
            token_id,
            pd: PhantomData::default(),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct AssetAmount {
    pub token_id: TokenId,
    pub amount: u64,
}

impl AssetAmount {
    pub fn coerce<T>(self) -> TypedAssetAmount<T> {
        TypedAssetAmount {
            token_id: self.token_id,
            amount: self.amount,
            pd: PhantomData::default(),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct TypedAssetAmount<T> {
    pub token_id: TokenId,
    pub amount: u64,
    pub pd: PhantomData<T>,
}

impl<T> TypedAssetAmount<T> {
    pub fn new(token_id: TokenId, amount: u64) -> Self {
        Self {
            token_id,
            amount,
            pd: PhantomData::default(),
        }
    }

    pub fn to_asset(self) -> TypedAsset<T> {
        TypedAsset {
            token_id: self.token_id,
            pd: PhantomData::default(),
        }
    }

    pub fn from_token(token: Token) -> Self {
        TypedAssetAmount::new(token.token_id, *token.amount.as_u64())
    }

    pub fn coerce<U>(self) -> TypedAssetAmount<U> {
        TypedAssetAmount {
            token_id: self.token_id,
            amount: self.amount,
            pd: PhantomData::default(),
        }
    }
}

impl<T> Add<TypedAssetAmount<T>> for TypedAssetAmount<T> {
    type Output = Self;

    fn add(self, rhs: TypedAssetAmount<T>) -> Self::Output {
        TypedAssetAmount {
            token_id: self.token_id,
            amount: self.amount + rhs.amount,
            pd: self.pd,
        }
    }
}

impl<T> Sub<TypedAssetAmount<T>> for TypedAssetAmount<T> {
    type Output = Self;

    fn sub(self, rhs: TypedAssetAmount<T>) -> Self::Output {
        TypedAssetAmount {
            token_id: self.token_id,
            amount: self.amount - rhs.amount,
            pd: self.pd,
        }
    }
}
