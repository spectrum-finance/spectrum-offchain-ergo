use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ProverError;

use spectrum_offchain::transaction::{TransactionCandidate, UnsignedTransactionOps};

pub trait SigmaProver {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, ProverError>;
}

pub struct NoopProver;

impl SigmaProver for NoopProver {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, ProverError> {
        Ok(tx.into_tx_without_proofs())
    }
}
