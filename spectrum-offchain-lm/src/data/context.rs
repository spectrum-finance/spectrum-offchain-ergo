use ergo_lib::ergotree_ir::chain::token::TokenId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct ExecutionContext {
    pub height: u32,
    pub mintable_token_id: TokenId,
    pub executor_prop: ErgoTree,
}
