use std::collections::HashMap;

use clap::Parser;
use derive_more::From;
use ergo_chain_sync::client::node::ErgoNodeHttpClient;
use ergo_lib::chain::contract::Contract;
use ergo_lib::chain::ergo_box::box_builder::{ErgoBoxCandidateBuilder, ErgoBoxCandidateBuilderError};
use ergo_lib::chain::ergo_state_context::ErgoStateContext;
use ergo_lib::chain::transaction::{Transaction, TxId, TxIoVec};
use ergo_lib::ergo_chain_types::Digest32;
use ergo_lib::ergotree_interpreter::sigma_protocol::private_input::{DlogProverInput, PrivateInput};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ContextExtension;
use ergo_lib::ergotree_ir::chain::address::{Address, AddressEncoder, NetworkPrefix};
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::{BoxValue, BoxValueError};
use ergo_lib::ergotree_ir::chain::ergo_box::{
    BoxId, BoxTokens, ErgoBox, ErgoBoxCandidate, NonMandatoryRegisterId, NonMandatoryRegisters,
};
use ergo_lib::ergotree_ir::chain::token::{Token, TokenAmount, TokenId};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::serialization::{SigmaParsingError, SigmaSerializable};
use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::{ProveDlog, SigmaProp};
use ergo_lib::wallet::box_selector::{BoxSelector, BoxSelectorError, SimpleBoxSelector};
use ergo_lib::wallet::miner_fee::{MINERS_FEE_ADDRESS, MINERS_FEE_BASE16_BYTES};
use ergo_lib::wallet::secret_key::SecretKey;
use ergo_lib::wallet::signing::{TransactionContext, TxSigningError};
use ergo_lib::wallet::tx_builder::{TxBuilder, TxBuilderError};
use isahc::prelude::Configurable;
use isahc::{AsyncReadResponseExt, HttpClient};
use serde::{Deserialize, Serialize};
use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};
use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox};
use spectrum_offchain::network::ErgoNetwork;
use spectrum_offchain_lm::data::assets::Lq;
use spectrum_offchain_lm::data::bundle::{StakingBundle, StakingBundleProto, BUNDLE_KEY_AMOUNT_USER};
use spectrum_offchain_lm::data::funding::DistributionFundingProto;
use spectrum_offchain_lm::data::miner::MinerOutput;
use spectrum_offchain_lm::data::redeemer::DepositOutput;
use spectrum_offchain_lm::data::PoolId;
use spectrum_offchain_lm::validators::{REDEEM_TEMPLATE, REDEEM_VALIDATOR, REDEEM_VALIDATOR_BYTES};
use thiserror::Error;

use ergo_chain_sync::client::types::{with_path, Url};
use spectrum_offchain::transaction::TransactionCandidate;
use spectrum_offchain_lm::data::pool::{Pool, ProgramConfig};
use spectrum_offchain_lm::ergo::{
    NanoErg, DEFAULT_MINER_FEE, MAX_VALUE, MIN_SAFE_BOX_VALUE, MIN_SAFE_FAT_BOX_VALUE,
};
use spectrum_offchain_lm::prover::{SeedPhrase, SigmaProver, Wallet, WalletSecret};

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

#[derive(Debug, Error, From)]
pub enum Error {
    #[error("box selector error: {0}")]
    BoxSelector(BoxSelectorError),
    #[error("box value error: {0}")]
    BoxValue(BoxValueError),
    #[error("box builder error: {0}")]
    ErgoBoxCandidateBuilder(ErgoBoxCandidateBuilderError),
    #[error("SigmaParsing error: {0}")]
    SigmaParse(SigmaParsingError),
    #[error("tx builder error: {0}")]
    TxBuilder(TxBuilderError),
    #[error("tx signing error: {0}")]
    TxSigning(TxSigningError),
    #[error("utxo error: {0:?}")]
    Utxo(UtxoError),
}

#[derive(Deserialize)]
pub struct DeployPoolConfig {
    node_addr: Url,
    http_client_timeout_duration_secs: u32,
    conf: ProgramConfig,
    tx_fee: BoxValue,
    erg_value_per_box: BoxValue,
    height: u32,
    initial_lq_token_deposit: TypedAssetAmount<Lq>,
    num_epochs_to_delegate: u64,
    operator_funding_secret: SeedPhrase,
}

#[tokio::main]
async fn main() {
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: DeployPoolConfig = serde_yaml::from_str(&raw_config).expect("Invalid configuration file");
    //

    //let (prover, funding_addr) = Wallet::try_from_seed(SeedPhrase::from(String::from(
    //    "gather obvious bracket ticket uphold quantum quit pistol math direct rural turn west youth acid",
    //)))
    //.expect("Invalid seed");

    let reward_token = Token {
        token_id: TokenId::from(
            Digest32::try_from(String::from(
                "991892666236da4771a8cdf95cf2c531daf1630ee0ee7805ebb24200f92ca886",
            ))
            .unwrap(),
        ),
        amount: TokenAmount::try_from(50000000000_u64).unwrap(),
    };
    let lq_token = Token {
        token_id: TokenId::from(
            Digest32::try_from(String::from(
                "20a6f717a998c73f6664b4f2c49d10dc05c5c8826776c566a2ab318a27b54e95",
            ))
            .unwrap(),
        ),
        amount: TokenAmount::try_from(1_u64).unwrap(),
    };
    // mint reward tokens
    //let reward_token_id = mint_token(50000000000, funding_addr.clone(), &prover).await;
    //println!("reward_token_id: {:?}", reward_token_id);
    //let lq_token_id = mint_token(50000000000, funding_addr, &prover).await;
    //println!("lq_token_id: {:?}", lq_token_id);

    let client = HttpClient::builder()
        .timeout(std::time::Duration::from_secs(50))
        .build()
        .unwrap();
    let explorer_url = Url::try_from(String::from("https://api.ergoplatform.com")).unwrap();
    let explorer = Explorer {
        client: client.clone(),
        base_url: explorer_url,
    };
    match deploy_pool(config, explorer).await {
        Ok(txs) => {
            let node_url = Url::try_from(String::from("http://213.239.193.208:9053")).unwrap();
            let node = ErgoNodeHttpClient::new(client, node_url);

            for tx in txs {
                println!("Submitting {:?} to node", tx.id());
                if let Err(e) = node.submit_tx(tx).await {
                    println!("ERROR SUBMITTING TO NODE: {:?}", e);
                } else {
                    println!("TX successfully submitted!");
                }
            }
        }
        Err(e) => {
            println!("DEPLOY POOL ERROR: {:?}", e);
        }
    }
}

pub async fn deploy_pool(config: DeployPoolConfig, explorer: Explorer) -> Result<Vec<Transaction>, Error> {
    let input = DeployPoolInputs::from(&config);
    let (prover, addr) = Wallet::try_from_seed(config.operator_funding_secret).expect("Invalid seed");
    let utxos = explorer.get_utxos(&addr).await;
    let txs = deploy_pool_chain_transaction(utxos, input, prover, addr)?;
    //dbg!(&txs);
    Ok(txs)
}

struct DeployPoolInputs {
    conf: ProgramConfig,
    tx_fee: BoxValue,
    erg_value_per_box: BoxValue,
    height: u32,
    initial_lq_token_deposit: TypedAssetAmount<Lq>,
    num_epochs_to_delegate: u64,
}

impl From<&DeployPoolConfig> for DeployPoolInputs {
    fn from(d: &DeployPoolConfig) -> Self {
        Self {
            conf: d.conf,
            tx_fee: d.tx_fee,
            erg_value_per_box: d.erg_value_per_box,
            height: d.height,
            initial_lq_token_deposit: d.initial_lq_token_deposit,
            num_epochs_to_delegate: d.num_epochs_to_delegate,
        }
    }
}

fn deploy_pool_chain_transaction(
    utxos: Vec<ErgoBox>,
    input: DeployPoolInputs,
    prover: Wallet,
    addr: Address,
) -> Result<Vec<Transaction>, Error> {
    //check_utxos(&utxos, &input)?;
    println!("UTXOs fine: only reward and LQ tokens found.");

    let DeployPoolInputs {
        conf,
        tx_fee,
        erg_value_per_box,
        height,
        initial_lq_token_deposit,
        num_epochs_to_delegate,
    } = input;
    let reward_token_budget = Token {
        token_id: conf.program_budget.token_id,
        amount: conf.program_budget.amount.try_into().unwrap(),
    };
    let guard = addr.script()?;
    let redeemer_prop = SigmaProp::from(ProveDlog::try_from(guard.clone()).unwrap());
    // We need to create a chain of 6 transactions:
    //   - TX 0: Move initial deposit of LQ and reward tokens into a single box. In addition move
    //     all other tokens in the UTXO set of the address to a separate box.
    //   - TX 1 to 3: Minting of pool NFT, vLQ tokens and TMP tokens.
    //   - TX 4: Initialize the pool-input box with tokens and parameters necessary to make the
    //     boxes in the next TX.
    //   - TX 5: Create first LM pool, staking bundle and redeemer out boxes.
    //
    // Now assuming that each created box will hold a value of `erg_value_per_box` the total amount
    // of ERG needed is:
    //   6 * tx_fee + 8 * erg_value_per_box + MIN_SAFE_FAT_BOX_VALUE
    //
    // Why 8 boxes of `erg_value_per_box`?
    //  - 1 box each for TX 0 to TX 4.
    //  - 3 boxes for TX 5
    // We have one box of value MIN_SAFE_FAT_BOX_VALUE to hold all other tokens that were picked up
    // by UTXO selection.

    // Since we're building a chain of transactions, we need to filter the output boxes of each
    // constituent transaction to be only those that are guarded by our wallet's key.
    let filter_tx_outputs = move |outputs: Vec<ErgoBox>| -> Vec<ErgoBox> {
        outputs
            .clone()
            .into_iter()
            .filter(|b| b.ergo_tree == guard)
            .collect()
    };

    // Let `i` denote the number of transactions left, then the target balance needed for the next
    // transaction is:
    //    (i-1)*(tx_fee + erg_value_per_box) + MINER_FEE + 3 * erg_value_per_box
    let calc_target_balance = |num_transactions_left| {
        assert!(num_transactions_left > 0);
        let b = (erg_value_per_box.checked_add(&tx_fee)?).checked_mul_u32(num_transactions_left - 1)?;
        let last_tx_amt = tx_fee.checked_add(&BoxValue::from(MIN_SAFE_FAT_BOX_VALUE).checked_mul_u32(3)?)?;
        b.checked_add(&last_tx_amt)
    };

    // Effect a single transaction that mints a token with given details, as described in comments
    // at the beginning. By default it uses `wallet_pk_ergo_tree` as the guard for the token box,
    // but this can be overriden with `different_token_box_guard`.
    let mint_token = |input_boxes: Vec<ErgoBox>,
                      num_transactions_left: &mut u32,
                      token_name,
                      token_desc,
                      token_amount|
     -> Result<(Token, Transaction), Error> {
        let target_balance = calc_target_balance(*num_transactions_left)?;
        let box_selector = SimpleBoxSelector::new();
        let box_selection = box_selector.select(input_boxes, target_balance, &[])?;
        let token = Token {
            token_id: box_selection.boxes.first().box_id().into(),
            amount: token_amount,
        };
        let ergo_tree = addr.script().unwrap();
        let mut builder = ErgoBoxCandidateBuilder::new(erg_value_per_box, ergo_tree.clone(), height);
        builder.mint_token(token.clone(), token_name, token_desc, 0);
        let mut output_candidates = vec![builder.build()?];

        let remaining_funds = ErgoBoxCandidateBuilder::new(
            calc_target_balance(*num_transactions_left - 1)?,
            ergo_tree,
            height,
        )
        .build()?;
        output_candidates.push(remaining_funds.clone());

        let miner_output = MinerOutput {
            erg_value: NanoErg::from(tx_fee),
        };
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

        let output_candidates = TxIoVec::from_vec(output_candidates.clone()).unwrap();
        let tx_candidate = TransactionCandidate {
            inputs,
            data_inputs: None,
            output_candidates,
        };
        let signed_tx = prover.sign(tx_candidate)?;

        *num_transactions_left -= 1;
        Ok((token, signed_tx))
    };

    // TX 0: transfer LQ and reward tokens to a single box -----------------------------------------
    let mut num_transactions_left = 6;
    let target_balance = calc_target_balance(num_transactions_left)?;

    let lq_token = Token {
        token_id: initial_lq_token_deposit.token_id,
        amount: TokenAmount::try_from(1_u64).unwrap(),
    };

    let box_selector = SimpleBoxSelector::new();
    let box_selection = box_selector.select(
        utxos.clone(),
        target_balance,
        &[reward_token_budget.clone(), lq_token.clone()],
    )?;

    let mut builder = ErgoBoxCandidateBuilder::new(erg_value_per_box, addr.script()?, height);
    builder.add_token(reward_token_budget.clone());
    builder.add_token(lq_token.clone());
    let lq_and_reward_box_candidate = builder.build()?;

    let remaining_funds = ErgoBoxCandidateBuilder::new(
        calc_target_balance(num_transactions_left - 1)?,
        addr.script()?,
        height,
    )
    .build()?;

    let miner_output = MinerOutput {
        erg_value: NanoErg::from(tx_fee),
    };
    let mut output_candidates = vec![
        lq_and_reward_box_candidate,
        remaining_funds,
        miner_output.clone().into_candidate(height),
    ];

    // If we have remaining reward and/or LQ tokens, preserve them in a separate box.
    let mut num_selected_reward_tokens = 0;
    let mut num_selected_lq_tokens = 0;
    for input_box in &box_selection.boxes {
        for token in input_box.tokens.iter().flatten() {
            if token.token_id == conf.program_budget.token_id {
                num_selected_reward_tokens += token.amount.as_u64();
            } else if token.token_id == initial_lq_token_deposit.token_id {
                num_selected_lq_tokens += token.amount.as_u64();
            } else {
                // There should be no other tokens here
                //unreachable!()
            }
        }
    }

    let mut remaining_tokens = vec![];
    if num_selected_reward_tokens > conf.program_budget.amount {
        let remaining_amount = num_selected_reward_tokens - conf.program_budget.amount;
        println!("{} remaining reward tokens from UTXOs", remaining_amount);
        remaining_tokens.push(Token {
            token_id: conf.program_budget.token_id,
            amount: TokenAmount::try_from(remaining_amount).unwrap(),
        });
    }

    if num_selected_lq_tokens > initial_lq_token_deposit.amount {
        let remaining_amount = num_selected_lq_tokens - initial_lq_token_deposit.amount;
        println!("{} remaining LQ tokens from UTXOs", remaining_amount);
        remaining_tokens.push(Token {
            token_id: initial_lq_token_deposit.token_id,
            amount: TokenAmount::try_from(remaining_amount).unwrap(),
        });
    }

    let funds_total = box_selection.boxes.iter().fold(NanoErg::from(0), |acc, ergobox| {
        acc + NanoErg::from(ergobox.value)
    });

    let accumulated_cost = NanoErg::from(target_balance);
    let funds_remain = funds_total.safe_sub(accumulated_cost);
    let mut builder = ErgoBoxCandidateBuilder::new(BoxValue::from(funds_remain), addr.script()?, height);
    for token in &remaining_tokens {
        builder.add_token(token.clone());
    }
    output_candidates.push(builder.build()?);

    let inputs = TxIoVec::from_vec(
        box_selection
            .boxes
            .clone()
            .into_iter()
            .map(|bx| (bx, ContextExtension::empty()))
            .collect::<Vec<_>>(),
    )
    .unwrap();

    let output_candidates = TxIoVec::from_vec(output_candidates).unwrap();
    let tx_candidate = TransactionCandidate {
        inputs,
        data_inputs: None,
        output_candidates,
    };
    num_transactions_left -= 1;
    let tx_0 = prover.sign(tx_candidate)?;

    for (i, o) in tx_0.outputs.iter().enumerate() {
        println!("#{}, value: {:?}", i, o.value);
    }
    println!("-----------------");
    // TX 1: Mint pool NFT -------------------------------------------------------------------------
    let inputs = if !remaining_tokens.is_empty() {
        let mut tx_0_outputs = tx_0.outputs.clone();
        let ix = tx_0_outputs
            .iter()
            .position(|ergobox| {
                if let Some(ref tokens) = ergobox.tokens {
                    tokens.iter().any(|t| remaining_tokens.contains(t))
                } else {
                    false
                }
            })
            .unwrap();
        let _ = tx_0_outputs.swap_remove(ix);
        filter_tx_outputs(tx_0_outputs)
    } else {
        filter_tx_outputs(tx_0.outputs.clone())
    };
    let (pool_nft, signed_mint_pool_nft_tx) = mint_token(
        inputs,
        &mut num_transactions_left,
        "".into(),
        "".into(),
        1.try_into().unwrap(),
    )?;

    // TX 2: Mint vLQ tokens -----------------------------------------------------------------------
    let inputs = filter_tx_outputs(signed_mint_pool_nft_tx.outputs.clone());
    let (vlq_tokens, signed_mint_vlq_tokens_tx) = mint_token(
        inputs,
        &mut num_transactions_left,
        "".into(),
        "".into(),
        MAX_VALUE.try_into().unwrap(),
    )?;

    // TX 3: Mint TMP tokens -----------------------------------------------------------------------
    let inputs = filter_tx_outputs(signed_mint_vlq_tokens_tx.outputs.clone());
    let (tmp_tokens, signed_mint_tmp_tokens_tx) = mint_token(
        inputs,
        &mut num_transactions_left,
        "".into(),
        "".into(),
        MAX_VALUE.try_into().unwrap(),
    )?;

    // TX 4: Create pool-input box -----------------------------------------------------------------
    let box_with_pool_nft =
        find_box_with_token(&signed_mint_pool_nft_tx.outputs, &pool_nft.token_id).unwrap();
    let box_with_rewards_tokens = find_box_with_token(&tx_0.outputs, &reward_token_budget.token_id).unwrap();
    let box_with_vlq_tokens =
        find_box_with_token(&signed_mint_vlq_tokens_tx.outputs, &vlq_tokens.token_id).unwrap();
    let box_with_tmp_tokens =
        find_box_with_token(&signed_mint_tmp_tokens_tx.outputs, &tmp_tokens.token_id).unwrap();

    let box_with_remaining_funds = signed_mint_tmp_tokens_tx.outputs[1].clone();

    let target_balance = calc_target_balance(num_transactions_left)?;
    let box_selector = SimpleBoxSelector::new();
    let box_selection = box_selector.select(
        vec![
            box_with_pool_nft,
            box_with_rewards_tokens,
            box_with_vlq_tokens,
            box_with_tmp_tokens,
            box_with_remaining_funds,
        ],
        target_balance,
        &[
            pool_nft.clone(),
            reward_token_budget.clone(),
            lq_token.clone(),
            vlq_tokens.clone(),
            tmp_tokens.clone(),
        ],
    )?;

    let inputs = TxIoVec::from_vec(
        box_selection
            .boxes
            .clone()
            .into_iter()
            .map(|bx| (bx, ContextExtension::empty()))
            .collect::<Vec<_>>(),
    )
    .unwrap();

    let mut pool_init_box_builder = ErgoBoxCandidateBuilder::new(erg_value_per_box, addr.script()?, height);
    pool_init_box_builder.add_token(pool_nft.clone());
    pool_init_box_builder.add_token(reward_token_budget.clone());
    pool_init_box_builder.add_token(lq_token.clone());
    pool_init_box_builder.add_token(vlq_tokens.clone());
    pool_init_box_builder.add_token(tmp_tokens.clone());
    pool_init_box_builder.set_register_value(NonMandatoryRegisterId::R4, <Vec<i32>>::from(conf).into());
    pool_init_box_builder.set_register_value(
        NonMandatoryRegisterId::R5,
        (conf.program_budget.amount as i64).into(),
    );
    pool_init_box_builder.set_register_value(
        NonMandatoryRegisterId::R6,
        (conf.max_rounding_error as i64).into(),
    );

    let pool_init_box = pool_init_box_builder.build()?;
    let remaining_funds = ErgoBoxCandidateBuilder::new(
        calc_target_balance(num_transactions_left - 1)?,
        addr.script()?,
        height,
    )
    .build()?;

    let mut box_of_consumed_tokens = ErgoBoxCandidateBuilder::new(
        BoxValue::try_from(4 * erg_value_per_box.as_u64()).unwrap(),
        addr.script()?,
        height,
    );

    let miner_output = MinerOutput {
        erg_value: NanoErg::from(tx_fee),
    };
    let output_candidates = vec![
        pool_init_box,
        remaining_funds,
        miner_output.into_candidate(height),
        box_of_consumed_tokens.build()?,
    ];

    let output_candidates = TxIoVec::from_vec(output_candidates).unwrap();
    let tx_candidate = TransactionCandidate {
        inputs,
        data_inputs: None,
        output_candidates,
    };
    let pool_input_tx = prover.sign(tx_candidate)?;

    println!("TX_4-----------------------");
    for (i, o) in pool_input_tx.outputs.iter().enumerate() {
        println!("#{}, value: {:?}", i, o.value);
    }
    num_transactions_left -= 1;

    // TX 5: Create first LM pool, stake bundle and redeemer out boxes -----------------------------
    let inputs = filter_tx_outputs(pool_input_tx.outputs.clone());
    let target_balance = calc_target_balance(num_transactions_left)?;

    let box_selector = SimpleBoxSelector::new();
    let box_selection = box_selector.select(
        inputs,
        target_balance,
        &[
            pool_nft.clone(),
            reward_token_budget.clone(),
            lq_token,
            vlq_tokens.clone(),
            tmp_tokens.clone(),
        ],
    )?;

    let inputs = TxIoVec::from_vec(
        box_selection
            .boxes
            .clone()
            .into_iter()
            .map(|bx| (bx, ContextExtension::empty()))
            .collect::<Vec<_>>(),
    )
    .unwrap();

    let lq_token_amount = initial_lq_token_deposit.amount;
    let vlq_token_amount = MAX_VALUE - lq_token_amount;

    let tmp_token_amount = MAX_VALUE - lq_token_amount * num_epochs_to_delegate;
    // Build LM pool box output candidate
    let pool = Pool {
        pool_id: PoolId::from(pool_nft.token_id),
        budget_rem: TypedAssetAmount::new(
            reward_token_budget.token_id.clone(),
            *reward_token_budget.amount.as_u64(),
        ),
        reserves_lq: TypedAssetAmount::new(
            initial_lq_token_deposit.token_id,
            initial_lq_token_deposit.amount,
        ),
        reserves_vlq: TypedAssetAmount::new(vlq_tokens.token_id.clone(), vlq_token_amount),
        reserves_tmp: TypedAssetAmount::new(tmp_tokens.token_id.clone(), tmp_token_amount),
        epoch_ix: None,
        conf,
        erg_value: MIN_SAFE_FAT_BOX_VALUE.into(),
    };

    let lm_pool_box_candidate = pool.into_candidate(height);

    let bundle_key_id: TokenId = inputs.first().0.box_id().into();

    let deposit_output = DepositOutput {
        bundle_key: TypedAssetAmount::new(bundle_key_id, BUNDLE_KEY_AMOUNT_USER),
        redeemer_prop: redeemer_prop.clone(),
        erg_value: MIN_SAFE_FAT_BOX_VALUE.into(),
        token_name: String::from(""),
        token_desc: String::from(""),
    };

    let deposit_output_candidate = deposit_output.into_candidate(height);

    // Build staking bundle candidate

    let staking_bundle = StakingBundleProto {
        bundle_key_id: TypedAsset::new(bundle_key_id),
        pool_id: PoolId::from(pool_nft.token_id),
        vlq: TypedAssetAmount::new(vlq_tokens.token_id, lq_token_amount),
        tmp: Some(TypedAssetAmount::new(
            tmp_tokens.token_id,
            lq_token_amount * num_epochs_to_delegate,
        )),
        redeemer_prop,
        erg_value: MIN_SAFE_FAT_BOX_VALUE.into(),
        token_name: String::from(""),
        token_desc: String::from(""),
    };
    let staking_bundle_candidate = staking_bundle.into_candidate(height);

    let mut miner_output = MinerOutput {
        erg_value: NanoErg::from(tx_fee),
    };
    let mut output_candidates = vec![
        lm_pool_box_candidate,
        deposit_output_candidate,
        staking_bundle_candidate,
    ];

    let funds_total = box_selection.boxes.iter().fold(NanoErg::from(0), |acc, ergobox| {
        acc + NanoErg::from(ergobox.value)
    });
    let accumulated_cost =
        NanoErg::from(3 * BoxValue::from(MIN_SAFE_FAT_BOX_VALUE).as_u64() + tx_fee.as_u64());
    let remaining_funds = funds_total - accumulated_cost;
    if remaining_funds >= MIN_SAFE_BOX_VALUE {
        let mut box_of_consumed_tokens =
            ErgoBoxCandidateBuilder::new(BoxValue::from(remaining_funds), addr.script()?, height);
        output_candidates.push(box_of_consumed_tokens.build()?);
    } else {
        miner_output.erg_value = miner_output.erg_value + remaining_funds;
    }

    output_candidates.push(miner_output.into_candidate(height));

    let output_candidates = TxIoVec::from_vec(output_candidates).unwrap();
    let tx_candidate = TransactionCandidate {
        inputs,
        data_inputs: None,
        output_candidates,
    };
    let init_pool_tx = prover.sign(tx_candidate)?;

    println!("TX_5-----------------------");
    for (i, o) in init_pool_tx.outputs.iter().enumerate() {
        println!("#{}, value: {:?}", i, o.value);
    }
    Ok(vec![
        tx_0,
        signed_mint_pool_nft_tx,
        signed_mint_vlq_tokens_tx,
        signed_mint_tmp_tokens_tx,
        pool_input_tx,
        init_pool_tx,
    ])
}

fn find_box_with_token(boxes: &Vec<ErgoBox>, token_id: &TokenId) -> Option<ErgoBox> {
    boxes
        .iter()
        .find(|&bx| {
            if let Some(tokens) = &bx.tokens {
                tokens.iter().any(|t| t.token_id == *token_id)
            } else {
                false
            }
        })
        .cloned()
}

#[derive(Debug)]
pub enum UtxoError {
    MissingLqToken,
    InsufficientRewardTokens {
        expected_quantity: u64,
        actual_quantity: u64,
    },
    OtherTokensInUtxos(Vec<Token>),
}

/// Returns true iff the UTXOs of the given wallet contain all necessary
fn check_utxos(utxos: &Vec<ErgoBox>, config: &DeployPoolInputs) -> Result<(), UtxoError> {
    let mut reward_tokens_count = 0_u64;
    let mut lq_token_found = false;
    let mut other_tokens = vec![];

    let expected_reward_token_quantity = config.conf.program_budget.amount;
    for ergobox in utxos {
        if let Some(ref tokens) = ergobox.tokens {
            for token in tokens {
                if token.token_id == config.conf.program_budget.token_id {
                    reward_tokens_count += *token.amount.as_u64();
                } else if token.token_id == config.initial_lq_token_deposit.token_id {
                    lq_token_found = true;
                } else {
                    other_tokens.push(token.clone());
                }
            }
        }
    }
    if !other_tokens.is_empty() {
        return Err(UtxoError::OtherTokensInUtxos(other_tokens));
    }

    if !lq_token_found {
        return Err(UtxoError::MissingLqToken);
    }
    if reward_tokens_count < config.conf.program_budget.amount {
        return Err(UtxoError::InsufficientRewardTokens {
            expected_quantity: expected_reward_token_quantity,
            actual_quantity: reward_tokens_count,
        });
    }
    Ok(())
}

/// Mint new token. Scans all input boxes to preserve any existing tokens.
async fn mint_token(token_amount: u64, wallet_addr: Address, wallet: &Wallet) -> TokenId {
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

    let target_balance = BoxValue::try_from(MIN_SAFE_BOX_VALUE).unwrap();
    let mut miner_output = MinerOutput {
        erg_value: DEFAULT_MINER_FEE,
    };
    let accumulated_cost = miner_output.erg_value + NanoErg::from(target_balance);
    let selection_value = BoxValue::try_from(accumulated_cost).unwrap();
    let box_selector = SimpleBoxSelector::new();
    let box_selection = box_selector.select(utxos, selection_value, &[]).unwrap();
    let token = Token {
        token_id: box_selection.boxes.first().box_id().into(),
        amount: TokenAmount::try_from(token_amount).unwrap(),
    };
    let token_id = token.token_id.clone();
    let ergo_tree = wallet_addr.script().unwrap();
    let mut builder = ErgoBoxCandidateBuilder::new(target_balance, ergo_tree.clone(), height);
    builder.mint_token(token.clone(), "".into(), "".into(), 10);
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

    let existing_tokens: Vec<_> = token_quantities
        .into_iter()
        .map(|(token_id, amount)| Token {
            token_id,
            amount: TokenAmount::try_from(amount).unwrap(),
        })
        .collect();

    let funds_remain = funds_total.safe_sub(accumulated_cost);
    if funds_remain >= MIN_SAFE_BOX_VALUE {
        let mut candidate = DistributionFundingProto {
            prop: ergo_tree,
            erg_value: funds_remain,
        }
        .into_candidate(height);
        assert!(candidate.tokens.is_none());
        candidate.tokens = Some(BoxTokens::from_vec(existing_tokens).unwrap());
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

    if let Err(e) = node.submit_tx(signed_tx).await {
        println!("ERGO NODE ERROR: {:?}", e);
    } else {
        println!("TX successfully submitted!");
    }

    token_id
}

#[derive(Parser)]
#[command(name = "spectrum-deploy-lm-pool")]
#[command(author = "Timothy Ling (@kettlebell) for Spectrum Finance")]
#[command(version = "0.1")]
#[command(about = "Spectrum Finance Liquidity Mining LM pool deployment tool", long_about = None)]
struct AppArgs {
    /// Path to the YAML configuration file.
    #[arg(long, short)]
    config_path: String,
}
