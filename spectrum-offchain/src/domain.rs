use std::marker::PhantomData;

use ergo_lib::ergotree_ir::chain::token::TokenId;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct TypedAsset<T> {
    pub token_id: TokenId,
    pub pd: PhantomData<T>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
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

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct TypedAssetAmount<T> {
    pub token_id: TokenId,
    pub amount: u64,
    pub pd: PhantomData<T>,
}
