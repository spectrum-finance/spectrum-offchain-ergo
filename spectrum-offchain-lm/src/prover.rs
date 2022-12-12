use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ProverError;

use spectrum_offchain::transaction::TransactionCandidate;

pub trait SigmaProver {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, ProverError>;
}
