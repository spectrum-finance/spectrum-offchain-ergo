use ergo_lib::chain::transaction::prover_result::ProverResult;
use ergo_lib::chain::transaction::{DataInput, Input, Transaction, TxIoVec};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::{ContextExtension, ProofBytes};
use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, ErgoBoxCandidate};

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct TransactionCandidate {
    pub inputs: TxIoVec<(ErgoBox, ContextExtension)>,
    /// inputs, that are not going to be spent by transaction, but will be reachable from inputs
    /// scripts. `dataInputs` scripts will not be executed, thus their scripts costs are not
    /// included in transaction cost and they do not contain spending proofs.
    pub data_inputs: Option<TxIoVec<DataInput>>,
    /// box candidates to be created by this transaction
    pub output_candidates: TxIoVec<ErgoBoxCandidate>,
}

impl TransactionCandidate {
    pub fn new(
        inputs: TxIoVec<(ErgoBox, ContextExtension)>,
        data_inputs: Option<TxIoVec<DataInput>>,
        output_candidates: TxIoVec<ErgoBoxCandidate>,
    ) -> Self {
        Self {
            inputs,
            data_inputs,
            output_candidates,
        }
    }
}

pub trait UnsignedTransactionOps {
    fn into_tx_without_proofs(self) -> Transaction;
}

impl UnsignedTransactionOps for TransactionCandidate {
    fn into_tx_without_proofs(self) -> Transaction {
        let empty_proofs_input = self.inputs.mapped_ref(|(i, ext)| {
            Input::new(
                i.box_id(),
                ProverResult {
                    proof: ProofBytes::Empty,
                    extension: ext.clone(),
                },
            )
        });

        // safe since the serialization error is impossible here
        // since we already serialized this unsigned tx (on calc tx id)
        Transaction::new(empty_proofs_input, self.data_inputs, self.output_candidates).unwrap()
    }
}
