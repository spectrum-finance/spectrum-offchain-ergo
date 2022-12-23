use ergo_lib::chain::ergo_state_context::ErgoStateContext;
use ergo_lib::chain::transaction::unsigned::UnsignedTransaction;
use ergo_lib::chain::transaction::{Transaction, UnsignedInput};
use ergo_lib::ergotree_interpreter::sigma_protocol::private_input::PrivateInput;
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::Prover;

use ergo_lib::ergotree_ir::chain::address::Address;
use ergo_lib::wallet::signing::{sign_transaction, TransactionContext, TxSigningError};
use spectrum_offchain::transaction::{TransactionCandidate, UnsignedTransactionOps};

pub trait SigmaProver {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, TxSigningError>;
}

pub struct Wallet {
    secrets: Vec<PrivateInput>,
    /// Necessary to use `sign_transaction` function from `sigma-rust`. If we're only signing P2PK
    /// inputs, then this field can be any arbitrary value.
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

impl SigmaProver for Wallet {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, TxSigningError> {
        let TransactionCandidate {
            inputs,
            data_inputs,
            output_candidates,
        } = tx;

        let mut unsigned_inputs = Vec::with_capacity(inputs.len());
        for (eb, extension) in &inputs {
            let addr = Address::recreate_from_ergo_tree(&eb.ergo_tree).unwrap();
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
        )
        .unwrap();
        let tx_context = TransactionContext::new(
            unsigned_tx,
            inputs.into_iter().map(|(b, _)| b).collect(),
            data_inputs.map(|d| d.to_vec()).unwrap_or_else(Vec::new),
        )?;
        sign_transaction(self, tx_context, &self.ergo_state_context, None)
    }
}

pub struct NoopProver;

impl SigmaProver for NoopProver {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, TxSigningError> {
        Ok(tx.into_tx_without_proofs())
    }
}

#[cfg(test)]
mod tests {
    use ergo_lib::{
        chain::{
            contract::Contract,
            ergo_box::box_builder::ErgoBoxCandidateBuilder,
            ergo_state_context::ErgoStateContext,
            transaction::{TxId, TxIoVec},
        },
        ergo_chain_types::{Header, PreHeader},
        ergotree_interpreter::sigma_protocol::{private_input::PrivateInput, prover::ContextExtension},
        ergotree_ir::{
            chain::{
                address::Address,
                ergo_box::{box_value::BoxValue, ErgoBox, NonMandatoryRegisters},
            },
            sigma_protocol::sigma_boolean::ProveDlog,
        },
        wallet::secret_key::SecretKey,
    };
    use sigma_test_util::force_any_val;
    use spectrum_offchain::transaction::TransactionCandidate;

    use super::{SigmaProver, Wallet};

    #[test]
    fn test_sigmaprover_sign() {
        let headers = force_any_val::<[Header; 10]>();
        let pre_header = PreHeader::from(headers.last().unwrap().clone());
        let ergo_state_context = ErgoStateContext { pre_header, headers };

        let secret_key = SecretKey::random_dlog();
        let address = secret_key.get_address_from_public_image();

        let wallet = Wallet {
            secrets: vec![PrivateInput::from(secret_key)],
            ergo_state_context,
        };

        let value = force_any_val::<BoxValue>();
        let input_box = ErgoBox::new(
            value,
            Contract::pay_to_address(&address).unwrap().ergo_tree(),
            None,
            NonMandatoryRegisters::empty(),
            0,
            force_any_val::<TxId>(),
            0,
        )
        .unwrap();
        let inputs = vec![(input_box, ContextExtension::empty())];

        let recipient = Address::P2Pk(force_any_val::<ProveDlog>());
        let output_box = ErgoBoxCandidateBuilder::new(
            BoxValue::try_from(value.as_u64() / 2).unwrap(),
            Contract::pay_to_address(&recipient).unwrap().ergo_tree(),
            0,
        )
        .build()
        .unwrap();

        let tx_candidate = TransactionCandidate {
            inputs: TxIoVec::from_vec(inputs).unwrap(),
            data_inputs: None,
            output_candidates: TxIoVec::from_vec(vec![output_box]).unwrap(),
        };

        assert!(wallet.sign(tx_candidate).is_ok());
    }
}
