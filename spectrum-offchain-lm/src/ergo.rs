use derive_more::{Add, Display, Div, From, Into, Mul, Sub, Sum};
use ergo_lib::chain::transaction::prover_result::ProverResult;
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::{ContextExtension, ProofBytes};
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;

/// Max amount of tokens allowed in Ergo.
pub const MAX_VALUE: u64 = 0x7fffffffffffffff;

pub fn empty_prover_result() -> ProverResult {
    ProverResult {
        proof: ProofBytes::Empty,
        extension: ContextExtension::empty(),
    }
}

#[derive(
    Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Sum, Add, Sub, Mul, Div, Into, From,
)]
pub struct NanoErg(u64);

impl From<BoxValue> for NanoErg {
    fn from(v: BoxValue) -> Self {
        Self(*v.as_u64())
    }
}

impl From<NanoErg> for BoxValue {
    fn from(nerg: NanoErg) -> Self {
        BoxValue::new(nerg.0).unwrap()
    }
}
