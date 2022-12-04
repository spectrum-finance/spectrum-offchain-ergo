use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use ergo_lib::ergotree_ir::chain::token::TokenId;

pub mod assets;
pub mod order;
pub mod bundle;

#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub struct OrderId(BoxId);

impl From<BoxId> for OrderId {
    fn from(bid: BoxId) -> Self {
        Self(bid)
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub struct PoolId(TokenId);

impl From<TokenId> for PoolId {
    fn from(tid: TokenId) -> Self {
        Self(tid)
    }
}
