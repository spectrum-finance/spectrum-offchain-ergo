use ergo_lib::chain::transaction::prover_result::ProverResult;
use ergo_lib::chain::transaction::unsigned::UnsignedTransaction;
use ergo_lib::chain::transaction::{Input, Transaction};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ProofBytes;

pub trait IntoTx {
    fn into_tx_without_proofs(self) -> Transaction;
}

impl IntoTx for UnsignedTransaction {
    fn into_tx_without_proofs(self) -> Transaction {
        let empty_proofs_input = self.inputs.mapped_ref(|ui| {
            Input::new(
                ui.box_id,
                ProverResult {
                    proof: ProofBytes::Empty,
                    extension: ui.extension.clone(),
                },
            )
        });

        // safe since the serialization error is impossible here
        // since we already serialized this unsigned tx (on calc tx id)
        Transaction::new(empty_proofs_input, self.data_inputs, self.output_candidates).unwrap()
    }
}
