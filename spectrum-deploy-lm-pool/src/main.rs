mod bip39_words;
mod generate_wallet;

use std::collections::HashMap;

use ergo_chain_sync::client::node::ErgoNodeHttpClient;
use ergo_lib::chain::ergo_box::box_builder::ErgoBoxCandidateBuilder;
use ergo_lib::chain::transaction::TxIoVec;
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ContextExtension;
use ergo_lib::ergotree_ir::chain::address::{Address, AddressEncoder, NetworkPrefix};
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
use ergo_lib::ergotree_ir::chain::token::{Token, TokenId};
use ergo_lib::wallet::box_selector::{BoxSelector, SimpleBoxSelector};
use isahc::prelude::Configurable;
use isahc::{AsyncReadResponseExt, HttpClient};
use serde::{Deserialize, Serialize};
use spectrum_offchain::event_sink::handlers::types::IntoBoxCandidate;
use spectrum_offchain::network::ErgoNetwork;
use spectrum_offchain_lm::data::funding::DistributionFundingProto;
use spectrum_offchain_lm::data::miner::MinerOutput;

use ergo_chain_sync::client::types::{with_path, Url};
use spectrum_offchain::transaction::TransactionCandidate;
use spectrum_offchain_lm::ergo::{NanoErg, DEFAULT_MINER_FEE, MIN_SAFE_BOX_VALUE};
use spectrum_offchain_lm::prover::{SeedPhrase, SigmaProver, Wallet};

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
                &format!(
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

#[tokio::main]
async fn main() {
    let (prover, from_addr) = Wallet::try_from_seed(SeedPhrase::from(String::from(
        //"gather obvious bracket ticket uphold quantum quit pistol math direct rural turn west youth acid",
        "tower twist supply size thing first track entire tenant kitten orbit cause file theme devote",
        //"victory hill video board mandate toss silver again fiber hard birth ritual claim man describe version domain soda caught target monster illegal camera soldier",
    )))
    .expect("Invalid seed 0");
    let (_, to_addr) = Wallet::try_from_seed(SeedPhrase::from(String::from(
        //"gather obvious bracket ticket uphold quantum quit pistol math direct rural turn west youth acid",
        "victory hill video board mandate toss silver again fiber hard birth ritual claim man describe version domain soda caught target monster illegal camera soldier",
    )))
    .expect("Invalid seed 0");
    transfer_erg(from_addr, to_addr, NanoErg::from(1_000_000_000_u64), &prover).await;
}

#[derive(Debug)]
pub enum UtxoError {
    InsufficientLqTokens,
    InsufficientRewardTokens {
        expected_quantity: u64,
        actual_quantity: u64,
    },
    OtherTokensInUtxos(Vec<Token>),
}

async fn transfer_erg(wallet_addr: Address, to_addr: Address, amt: NanoErg, wallet: &Wallet) {
    let client = HttpClient::builder()
        .timeout(std::time::Duration::from_secs(50))
        .build()
        .unwrap();

    let node_url = Url::try_from(String::from("http://213.239.193.208:9053")).unwrap();
    let explorer_url = Url::try_from(String::from("https://api.ergoplatform.com")).unwrap();
    let explorer = Explorer {
        client: client.clone(),
        base_url: explorer_url,
    };
    let node = ErgoNodeHttpClient::new(client, node_url);

    let utxos = explorer.get_utxos(&wallet_addr).await;
    let height = node.get_height().await;

    let target_balance = BoxValue::from(amt);
    let mut miner_output = MinerOutput {
        erg_value: DEFAULT_MINER_FEE,
    };
    let accumulated_cost = miner_output.erg_value + NanoErg::from(target_balance);
    let selection_value = BoxValue::try_from(accumulated_cost).unwrap();
    let box_selector = SimpleBoxSelector::new();
    let box_selection = box_selector.select(utxos, selection_value, &[]).unwrap();
    let to_ergo_tree = to_addr.script().unwrap();
    let builder = ErgoBoxCandidateBuilder::new(target_balance, to_ergo_tree.clone(), height);
    let mut output_candidates = vec![builder.build().unwrap()];

    let funds_total = box_selection.boxes.iter().fold(NanoErg::from(0), |acc, ergobox| {
        acc + NanoErg::from(ergobox.value)
    });

    let mut token_quantities: HashMap<TokenId, u64> = HashMap::new();

    for ergobox in &box_selection.boxes {
        for t in ergobox.tokens.iter().flatten() {
            *token_quantities.entry(t.token_id).or_insert(0) += t.amount.as_u64();
        }
    }

    let funds_remain = funds_total.safe_sub(accumulated_cost);
    if funds_remain >= MIN_SAFE_BOX_VALUE {
        let from_ergo_tree = wallet_addr.script().unwrap();
        let mut candidate = DistributionFundingProto {
            prop: from_ergo_tree,
            erg_value: funds_remain,
        }
        .into_candidate(height);
        assert!(candidate.tokens.is_none());
        candidate.tokens = None; //Some(BoxTokens::from_vec(existing_tokens).unwrap());
        output_candidates.push(candidate);
    } else {
        miner_output.erg_value = miner_output.erg_value + funds_remain;
    }
    output_candidates.push(miner_output.into_candidate(height));
    let inputs = TxIoVec::from_vec(
        box_selection
            .boxes
            .clone()
            .into_iter()
            .map(|bx| (bx, ContextExtension::empty()))
            .collect::<Vec<_>>(),
    )
    .unwrap();

    let tx_candidate = TransactionCandidate {
        inputs,
        data_inputs: None,
        output_candidates: TxIoVec::from_vec(output_candidates).unwrap(),
    };
    let signed_tx = wallet.sign(tx_candidate).unwrap();
    dbg!(&signed_tx);

    let tx_id = signed_tx.id();
    if let Err(e) = node.submit_tx(signed_tx).await {
        println!("ERGO NODE ERROR: {:?}", e);
    } else {
        println!("TX {:?} successfully submitted!", tx_id);
    }
}
