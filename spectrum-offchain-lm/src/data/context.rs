use std::fmt::{Display, Formatter, Write};

use ergo_lib::ergotree_ir::chain::token::TokenId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct ExecutionContext {
    pub height: u32,
    pub mintable_token_id: TokenId,
    pub executor_prop: ErgoTree,
}

impl Display for ExecutionContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*format!(
            "Context[height={}, mintable_token_id={:?}, executor_prop={}]",
            self.height,
            self.mintable_token_id,
            base16::encode_lower(&self.executor_prop.sigma_serialize_bytes().unwrap())
        ))
    }
}
