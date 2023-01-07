use ergo_lib::chain::transaction::{Transaction, TxIoVec};
use ergo_lib::ergotree_interpreter::sigma_protocol::private_input::DlogProverInput;
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ContextExtension;
use ergo_lib::ergotree_ir::chain::address::{Address, AddressEncoder, NetworkPrefix};
use ergo_lib::ergotree_ir::chain::ergo_box::{BoxTokens, ErgoBox, ErgoBoxCandidate, NonMandatoryRegisters};
use ergo_lib::ergotree_ir::chain::token::{Token, TokenAmount, TokenId};
use ergo_lib::wallet::miner_fee::MINERS_FEE_ADDRESS;
use isahc::{AsyncReadResponseExt, HttpClient};
use serde::{Deserialize, Serialize};

use ergo_chain_sync::client::types::{with_path, Url};
use spectrum_offchain::transaction::TransactionCandidate;
use spectrum_offchain_lm::data::pool::ProgramConfig;
use spectrum_offchain_lm::ergo::{DEFAULT_MINER_FEE, MAX_VALUE, MIN_SAFE_BOX_VALUE};
use spectrum_offchain_lm::prover::{SigmaProver, Wallet, WalletSecret};

pub struct Explorer {
    pub client: HttpClient,
    pub base_url: Url,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Items<A> {
    items: Vec<A>,
}

impl Explorer {
    pub async fn get_utxos(&self, addr: &Address) -> Vec<ErgoBox> {
        self.client
            .get_async(with_path(
                &self.base_url,
                &*format!(
                    "/api/v1/boxes/unspent/byAddress/{}",
                    AddressEncoder::encode_address_as_string(NetworkPrefix::Mainnet, addr),
                ),
            ))
            .await
            .ok()
            .unwrap()
            .json::<Items<ErgoBox>>()
            .await
            .unwrap()
            .items
    }
}

async fn mint(amount: u64, wallet: WalletSecret, explorer: Explorer, height: u32) -> Transaction {
    let erg_target = MIN_SAFE_BOX_VALUE + DEFAULT_MINER_FEE;
    let addr = Address::P2Pk(DlogProverInput::from(wallet.clone()).public_image());
    let utxos = explorer.get_utxos(&addr).await;
    let mut inputs: Vec<ErgoBox> = Vec::new();
    for bx in utxos {
        let acc = inputs.iter().fold(0u64, |acc, i| acc + i.value.as_u64());
        if acc < <u64>::from(erg_target) {
            inputs.push(bx)
        } else {
            break;
        }
    }
    let total_input_erg = inputs.iter().fold(0u64, |acc, i| acc + i.value.as_u64());
    let mut input_tokens = Vec::new();
    for i in &inputs {
        for tk in i
            .tokens
            .as_ref()
            .map(|tk| tk.iter().collect::<Vec<_>>())
            .unwrap_or(Vec::new())
        {
            input_tokens.push(tk.clone())
        }
    }
    let mintable_id: TokenId = inputs[0].box_id().into();
    let mut output_tokens = input_tokens;
    output_tokens.push(Token {
        token_id: mintable_id,
        amount: TokenAmount::try_from(MAX_VALUE).unwrap(),
    });
    let minting_out = ErgoBoxCandidate {
        value: (total_input_erg - <u64>::from(DEFAULT_MINER_FEE))
            .try_into()
            .unwrap(),
        ergo_tree: addr.script().unwrap(),
        tokens: Some(BoxTokens::from_vec(output_tokens).unwrap()),
        additional_registers: NonMandatoryRegisters::empty(),
        creation_height: height,
    };
    let miner_out = ErgoBoxCandidate {
        value: DEFAULT_MINER_FEE.into(),
        ergo_tree: MINERS_FEE_ADDRESS.script().unwrap(),
        tokens: None,
        additional_registers: NonMandatoryRegisters::empty(),
        creation_height: height,
    };
    let tx = TransactionCandidate::new(
        TxIoVec::from_vec(
            inputs
                .into_iter()
                .map(|bx| (bx, ContextExtension::empty()))
                .collect(),
        )
        .unwrap(),
        None,
        TxIoVec::from_vec(vec![minting_out, miner_out]).unwrap(),
    );
    let prover = Wallet::trivial(vec![wallet]);
    prover.sign(tx).unwrap()
}

async fn deploy_pool(conf: ProgramConfig, wallet: WalletSecret, explorer: Explorer) -> Vec<Transaction> {
    vec![]
}
