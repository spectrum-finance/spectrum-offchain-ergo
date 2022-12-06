use ergo_lib::ergotree_ir::chain::token::TokenId;

#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub struct ExecutionContext {
    pub height: u32,
    pub mintable_token_id: TokenId,
}
