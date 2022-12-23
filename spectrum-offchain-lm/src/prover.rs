use ergo_lib::chain::ergo_state_context::ErgoStateContext;
use ergo_lib::chain::transaction::unsigned::UnsignedTransaction;
use ergo_lib::chain::transaction::{Transaction, TransactionError, UnsignedInput};
use ergo_lib::ergotree_interpreter::sigma_protocol::private_input::PrivateInput;
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::Prover;

use ergo_lib::ergotree_ir::chain::address::{Address, AddressError};
use ergo_lib::wallet::signing::{sign_transaction, TransactionContext, TxSigningError};
use spectrum_offchain::transaction::{TransactionCandidate, UnsignedTransactionOps};
use thiserror::Error;

pub trait SigmaProver {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, SigmaProverError>;
}

pub struct Wallet {
    secrets: Vec<PrivateInput>,
    /// Necessary to use `sign_transaction` function from `sigma-rust`. This field needs to be
    /// refreshed periodically with the latest Headers.
    ergo_state_context: ErgoStateContext,
}

impl Prover for Wallet {
    fn secrets(&self) -> &[PrivateInput] {
        self.secrets.as_ref()
    }

    fn append_secret(&mut self, input: PrivateInput) {
        self.secrets.push(input);
    }
}

#[derive(Error, Debug)]
pub enum SigmaProverError {
    #[error("Address error: ({0})")]
    Address(#[from] AddressError),
    #[error("Transaction error: ({0})")]
    Transaction(#[from] TransactionError),
    #[error("Transaction signing error: ({0})")]
    TxSigning(#[from] TxSigningError),
}

impl SigmaProver for Wallet {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, SigmaProverError> {
        let TransactionCandidate {
            inputs,
            data_inputs,
            output_candidates,
        } = tx;

        let mut unsigned_inputs = Vec::with_capacity(inputs.len());
        for (eb, extension) in &inputs {
            let addr = Address::recreate_from_ergo_tree(&eb.ergo_tree)?;
            if let Address::P2Pk(_) = addr {
                unsigned_inputs.push(UnsignedInput {
                    box_id: eb.box_id(),
                    extension: extension.clone(),
                });
            }
        }

        let unsigned_tx = UnsignedTransaction::new_from_vec(
            unsigned_inputs,
            data_inputs
                .clone()
                .map(|d| d.mapped(|b| b.box_id().into()).to_vec())
                .unwrap_or_else(Vec::new),
            output_candidates.to_vec(),
        )?;
        let tx_context = TransactionContext::new(
            unsigned_tx,
            inputs.into_iter().map(|(b, _)| b).collect(),
            data_inputs.map(|d| d.to_vec()).unwrap_or_else(Vec::new),
        )?;
        sign_transaction(self, tx_context, &self.ergo_state_context, None)
            .map_err(SigmaProverError::TxSigning)
    }
}

pub struct NoopProver;

impl SigmaProver for NoopProver {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, SigmaProverError> {
        Ok(tx.into_tx_without_proofs())
    }
}
