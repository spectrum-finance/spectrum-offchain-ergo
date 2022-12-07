use ergo_lib::chain::transaction::prover_result::ProverResult;
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::{ContextExtension, ProofBytes};

/// Max amount of tokens allowed in Ergo.
pub const MAX_VALUE: u64 = 0x7fffffffffffffff;

pub fn empty_prover_result() -> ProverResult {
    ProverResult {
        proof: ProofBytes::Empty,
        extension: ContextExtension::empty(),
    }
}
