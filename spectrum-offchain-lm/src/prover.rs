use std::rc::Rc;

use derive_more::Into;
use ergo_lib::chain::ergo_state_context::ErgoStateContext;
use ergo_lib::chain::transaction::prover_result::ProverResult;
use ergo_lib::chain::transaction::unsigned::UnsignedTransaction;
use ergo_lib::chain::transaction::{Input, Transaction, UnsignedInput};
use ergo_lib::ergotree_interpreter::eval::env::Env;
use ergo_lib::ergotree_interpreter::sigma_protocol::private_input::{DlogProverInput, PrivateInput};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::hint::HintsBag;
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::{ProofBytes, Prover};
use ergo_lib::ergotree_ir::chain::address::Address;
use ergo_lib::wallet::signing::{make_context, sign_transaction, TransactionContext, TxSigningError};
use serde::Deserialize;
use sigma_test_util::force_any_val;

use spectrum_offchain::transaction::{TransactionCandidate, UnsignedTransactionOps};

pub trait SigmaProver {
    fn sign(&self, tx: TransactionCandidate) -> Result<Transaction, TxSigningError>;
}

#[derive(Clone, Deserialize, Into)]
#[serde(try_from = "String")]
pub struct WalletSecret(DlogProverInput);

impl TryFrom<String> for WalletSecret {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        DlogProverInput::from_base16_str(value)
            .map(Self)
            .ok_or(format!("Private inputs must be provided in Base16"))
    }
}

pub struct Wallet {
    secrets: Vec<PrivateInput>,
    /// Necessary to use `sign_transaction` function from `sigma-rust`. If we're only signing P2PK
    /// inputs, then this field can be any arbitrary value.
    ergo_state_context: ErgoStateContext,
}

impl Wallet {
    pub fn trivial(secrets: Vec<WalletSecret>) -> Self {
        Self {
            secrets: secrets
                .into_iter()
                .map(|WalletSecret(pi)| PrivateInput::DlogProverInput(pi))
                .collect(),
            ergo_state_context: force_any_val(),
        }
    }
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

        let unsigned_inputs = inputs
            .iter()
            .map(|(bx, ext)| UnsignedInput {
                box_id: bx.box_id(),
                extension: ext.clone(),
            })
            .collect();

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
        let tx = tx_context.spending_tx.clone();
        let message_to_sign = tx.bytes_to_sign()?;
        let signed_inputs = tx.inputs.enumerated().try_mapped(|(idx, input)| {
            let input_box = tx_context
                .get_input_box(&input.box_id)
                .ok_or(TxSigningError::InputBoxNotFound(idx))?;
            let addr = Address::recreate_from_ergo_tree(&input_box.ergo_tree).unwrap();
            if let Address::P2Pk(_) = addr {
                let ctx = Rc::new(make_context(&self.ergo_state_context, &tx_context, idx)?);
                let hints_bag = HintsBag::empty();
                self.prove(
                    &input_box.ergo_tree,
                    &Env::empty(),
                    ctx,
                    message_to_sign.as_slice(),
                    &hints_bag,
                )
                .map(|proof| Input::new(input.box_id, proof.into()))
                .map_err(|e| TxSigningError::ProverError(e, idx))
            } else {
                Ok(Input::new(
                    input_box.box_id(),
                    ProverResult {
                        proof: ProofBytes::Empty,
                        extension: input.extension.clone(),
                    },
                ))
            }
        })?;
        Ok(Transaction::new(
            signed_inputs,
            tx.data_inputs,
            tx.output_candidates,
        )?)
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
