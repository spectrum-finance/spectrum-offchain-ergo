mod bip39_words;
mod generate_wallet;

use std::ops::Add;

use clap::{Parser, Subcommand};
use derive_more::From;
use ergo_chain_sync::client::node::ErgoNodeHttpClient;

use ergo_lib::chain::ergo_box::box_builder::{ErgoBoxCandidateBuilder, ErgoBoxCandidateBuilderError};

use ergo_lib::chain::transaction::{Transaction, TxIoVec};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ContextExtension;
use ergo_lib::ergotree_ir::chain::address::{Address, AddressEncoder, NetworkPrefix};
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::{BoxValue, BoxValueError};
use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, NonMandatoryRegisterId};
use ergo_lib::ergotree_ir::chain::token::{Token, TokenAmount, TokenId};
use ergo_lib::ergotree_ir::serialization::SigmaParsingError;
use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::{ProveDlog, SigmaProp};
use ergo_lib::wallet::box_selector::{BoxSelector, BoxSelectorError, SimpleBoxSelector};
use ergo_lib::wallet::signing::TxSigningError;
use ergo_lib::wallet::tx_builder::TxBuilderError;
use isahc::prelude::Configurable;
use isahc::HttpClient;
use serde::Deserialize;
use spectrum_deploy_lm_pool::{Explorer, ExplorerError};
use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};
use spectrum_offchain::event_sink::handlers::types::IntoBoxCandidate;
use spectrum_offchain::network::ErgoNetwork;
use spectrum_offchain_lm::data::assets::Lq;
use spectrum_offchain_lm::data::bundle::{StakingBundleProto, BUNDLE_KEY_AMOUNT_USER};
use spectrum_offchain_lm::data::miner::MinerOutput;
use spectrum_offchain_lm::data::redeemer::DepositOutput;
use spectrum_offchain_lm::data::PoolId;
use thiserror::Error;

use ergo_chain_sync::client::types::Url;
use spectrum_offchain::transaction::TransactionCandidate;
use spectrum_offchain_lm::data::pool::{Pool, ProgramConfig};
use spectrum_offchain_lm::ergo::{NanoErg, MAX_VALUE, MIN_SAFE_BOX_VALUE, MIN_SAFE_FAT_BOX_VALUE};
use spectrum_offchain_lm::prover::{SeedPhrase, SigmaProver, Wallet};

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
    #[error("pool validation error: {0:?}")]
    PoolValidation(PoolValidationError),
    #[error("explorer error: {0:?}")]
    Explorer(ExplorerError),
}

#[derive(Deserialize)]
pub struct DeployPoolConfig {
    node_addr: Url,
    http_client_timeout_duration_secs: u32,
    conf: ProgramConfig,
    tx_fee: BoxValue,
    erg_value_per_box: BoxValue,
    initial_lq_token_deposit: TypedAssetAmount<Lq>,
    num_epochs_to_delegate: u64,
    operator_funding_secret: SeedPhrase,
}

#[tokio::main]
async fn main() {
    let args = AppArgs::parse();

    match args.command {
        Command::DeployPool { config_path } => {
            let raw_config = std::fs::read_to_string(config_path).expect("Cannot load configuration file");
            let config: DeployPoolConfig =
                serde_yaml::from_str(&raw_config).expect("Invalid configuration file");

            let client = HttpClient::builder()
                .timeout(std::time::Duration::from_secs(
                    config.http_client_timeout_duration_secs as u64,
                ))
                .build()
                .unwrap();
            let explorer_url = Url::try_from(String::from("https://api.ergoplatform.com")).unwrap();
            let explorer = Explorer {
                client: client.clone(),
                base_url: explorer_url,
            };
            let node = ErgoNodeHttpClient::new(client, config.node_addr.clone());
            match deploy_pool(config, &node, explorer).await {
                Ok((txs, pool_id)) => {
                    for (tx, description) in txs {
                        let tx_id = tx.id();
                        if let Err(e) = node.submit_tx(tx).await {
                            println!("ERROR SUBMITTING TO NODE: {:?}", e);
                            break;
                        } else {
                            println!(
                                "TX {:?} successfully submitted! (Description: {})",
                                tx_id, description
                            );
                        }
                    }
                    println!(
                        "Program ID is: {:?}. Please note it down to whitelist your program",
                        pool_id
                    );
                }
                Err(e) => {
                    println!("DEPLOY POOL ERROR: {:?}", e);
                }
            }
        }

        Command::GenerateNewWallet => {
            generate_new_wallet();
        }
    }
}

pub async fn deploy_pool(
    config: DeployPoolConfig,
    node: &ErgoNodeHttpClient,
    explorer: Explorer,
) -> Result<(Vec<(Transaction, String)>, PoolId), Error> {
    let input = DeployPoolInputs::from(&config);
    let (prover, addr) = Wallet::try_from_seed(config.operator_funding_secret).expect("Invalid seed");
    println!(
        "Wallet address: {:?}",
        AddressEncoder::encode_address_as_string(NetworkPrefix::Mainnet, &addr)
    );
    let current_height = node.get_height().await;
    validate_pool(&input, current_height)?;
    let utxos = explorer.get_utxos(&addr).await?;
    let res = deploy_pool_chain_transaction(utxos, input, current_height, prover, addr)?;
    //dbg!(&txs);
    Ok(res)
}

struct DeployPoolInputs {
    conf: ProgramConfig,
    tx_fee: BoxValue,
    erg_value_per_box: BoxValue,
    initial_lq_token_deposit: TypedAssetAmount<Lq>,
    num_epochs_to_delegate: u64,
}

impl From<&DeployPoolConfig> for DeployPoolInputs {
    fn from(d: &DeployPoolConfig) -> Self {
        Self {
            conf: d.conf,
            tx_fee: d.tx_fee,
            erg_value_per_box: d.erg_value_per_box,
            initial_lq_token_deposit: d.initial_lq_token_deposit,
            num_epochs_to_delegate: d.num_epochs_to_delegate,
        }
    }
}

fn deploy_pool_chain_transaction(
    utxos: Vec<ErgoBox>,
    input: DeployPoolInputs,
    height: u32,
    prover: Wallet,
    addr: Address,
) -> Result<(Vec<(Transaction, String)>, PoolId), Error> {
    check_utxos(&utxos, &input)?;
    println!("UTXOs fine: only reward and LQ tokens found.");

    println!("UTXOS---------");
    for ergo_box in &utxos {
        println!(
            "   {}: value: {:?}, #tokens: {}",
            ergo_box.box_id(),
            ergo_box.value,
            ergo_box.tokens.as_ref().map(|t| t.len()).unwrap_or(0)
        );
    }

    let DeployPoolInputs {
        conf,
        tx_fee,
        erg_value_per_box,
        initial_lq_token_deposit,
        num_epochs_to_delegate,
        ..
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
    //   6 * tx_fee + 5 * erg_value_per_box + 3 * MIN_SAFE_FAT_BOX_VALUE
    //
    //  - 1 box each of `erg_value_per_box` for TX 0 to TX 4.
    //  - 3 boxes of `MIN_SAFE_FAT_BOX_VALUE` for TX 5
    //

    // Since we're building a chain of transactions, we need to filter the output boxes of each
    // constituent transaction to be only those that are guarded by our wallet's key.
    let filter_tx_outputs = move |outputs: Vec<ErgoBox>| -> Vec<ErgoBox> {
        outputs.into_iter().filter(|b| b.ergo_tree == guard).collect()
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
        println!("Mint token");
        let target_balance = calc_target_balance(*num_transactions_left)?;
        println!("  Target balance: {:?}", target_balance);
        let box_selector = SimpleBoxSelector::new();
        let box_selection = box_selector.select(input_boxes, target_balance, &[])?;
        println!("  Box selection:");
        for ergo_box in &box_selection.boxes {
            println!(
                "   {}: value: {:?}, #tokens: {}",
                ergo_box.box_id(),
                ergo_box.value,
                ergo_box.tokens.as_ref().map(|t| t.len()).unwrap_or(0)
            );
        }
        let token = Token {
            token_id: box_selection.boxes.first().box_id().into(),
            amount: token_amount,
        };
        let ergo_tree = addr.script().unwrap();
        let mut builder = ErgoBoxCandidateBuilder::new(erg_value_per_box, ergo_tree.clone(), height);
        builder.mint_token(token.clone(), token_name, token_desc, 0);
        let mut output_candidates = vec![builder.build()?];

        let accumulated_cost = *erg_value_per_box.as_u64() + tx_fee.as_u64();

        let miner_output = MinerOutput {
            erg_value: NanoErg::from(tx_fee),
        };
        output_candidates.push(miner_output.into_candidate(height));

        let input_funds_total = box_selection.boxes.iter().fold(NanoErg::from(0), |acc, ergobox| {
            acc + NanoErg::from(ergobox.value)
        });

        let funds_remaining = input_funds_total.safe_sub(NanoErg::from(accumulated_cost));
        let funds_remaining_out_box =
            ErgoBoxCandidateBuilder::new(BoxValue::from(funds_remaining), ergo_tree, height).build()?;
        output_candidates.push(funds_remaining_out_box);
        let inputs = TxIoVec::from_vec(
            box_selection
                .boxes
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

        println!("  outputs:");
        let mut output_total = 0;
        for ergo_box in &signed_tx.outputs {
            println!(
                "    {}: value: {:?}, #tokens: {}",
                ergo_box.box_id(),
                ergo_box.value,
                ergo_box.tokens.as_ref().map(|t| t.len()).unwrap_or(0)
            );
            output_total += ergo_box.value.as_u64();
        }

        assert_eq!(input_funds_total, NanoErg::from(output_total));

        *num_transactions_left -= 1;
        Ok((token, signed_tx))
    };

    // TX 0: transfer LQ and reward tokens to a single box -----------------------------------------
    let mut num_transactions_left = 6;
    let target_balance = calc_target_balance(num_transactions_left)?;

    println!("TX 0: target_balance: {:?}", target_balance);

    let lq_token = Token {
        token_id: initial_lq_token_deposit.token_id,
        amount: TokenAmount::try_from(input.initial_lq_token_deposit.amount).unwrap(),
    };

    let box_selector = SimpleBoxSelector::new();
    let box_selection = box_selector.select(
        utxos,
        target_balance,
        &[reward_token_budget.clone(), lq_token.clone()],
    )?;

    let mut builder = ErgoBoxCandidateBuilder::new(erg_value_per_box, addr.script()?, height);
    builder.add_token(reward_token_budget.clone());
    builder.add_token(lq_token.clone());
    let lq_and_reward_box_candidate = builder.build()?;

    let miner_output = MinerOutput {
        erg_value: NanoErg::from(tx_fee),
    };
    let mut output_candidates = vec![lq_and_reward_box_candidate, miner_output.into_candidate(height)];

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
                unreachable!("There should only be reward and LQ tokens for specified wallet!")
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

    let mut accumulated_cost = NanoErg::from(erg_value_per_box) + NanoErg::from(tx_fee);

    if !remaining_tokens.is_empty() {
        accumulated_cost = accumulated_cost.add(MIN_SAFE_BOX_VALUE);
        let mut builder =
            ErgoBoxCandidateBuilder::new(BoxValue::from(MIN_SAFE_BOX_VALUE), addr.script()?, height);
        for token in &remaining_tokens {
            builder.add_token(token.clone());
        }
        output_candidates.push(builder.build()?);
    }

    let funds_total = box_selection.boxes.iter().fold(NanoErg::from(0), |acc, ergobox| {
        acc + NanoErg::from(ergobox.value)
    });

    let funds_remain = funds_total.safe_sub(accumulated_cost);
    let builder = ErgoBoxCandidateBuilder::new(BoxValue::from(funds_remain), addr.script()?, height);
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
    let mut output_total = 0;
    println!("TX_0 outputs:");
    for ergo_box in &tx_0.outputs {
        println!(
            "   {}: value: {:?}, #tokens: {}",
            ergo_box.box_id(),
            ergo_box.value,
            ergo_box.tokens.as_ref().map(|t| t.len()).unwrap_or(0)
        );
        output_total += ergo_box.value.as_u64();
    }

    assert_eq!(funds_total, NanoErg::from(output_total));

    // TX 1: Mint pool NFT -------------------------------------------------------------------------
    let inputs = if !remaining_tokens.is_empty() {
        let mut tx_0_outputs = tx_0.outputs.clone().to_vec();
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
        filter_tx_outputs(tx_0.outputs.clone().to_vec())
    };
    println!("TX_1: pool NFT");
    let (pool_nft, signed_mint_pool_nft_tx) = mint_token(
        inputs,
        &mut num_transactions_left,
        "".into(),
        "".into(),
        1.try_into().unwrap(),
    )?;

    // TX 2: Mint vLQ tokens -----------------------------------------------------------------------
    let inputs = filter_tx_outputs(signed_mint_pool_nft_tx.outputs.clone().to_vec());
    println!("TX_2: vLQ tokens");
    let (vlq_tokens, signed_mint_vlq_tokens_tx) = mint_token(
        inputs,
        &mut num_transactions_left,
        "".into(),
        "".into(),
        MAX_VALUE.try_into().unwrap(),
    )?;

    // TX 3: Mint TMP tokens -----------------------------------------------------------------------
    let inputs = filter_tx_outputs(signed_mint_vlq_tokens_tx.outputs.clone().to_vec());
    println!("TX_3: TMP tokens");
    let (tmp_tokens, signed_mint_tmp_tokens_tx) = mint_token(
        inputs,
        &mut num_transactions_left,
        "".into(),
        "".into(),
        MAX_VALUE.try_into().unwrap(),
    )?;

    // TX 4: Create pool-input box -----------------------------------------------------------------
    let box_with_pool_nft =
        find_box_with_token(signed_mint_pool_nft_tx.outputs.as_vec(), &pool_nft.token_id).unwrap();
    let box_with_rewards_tokens =
        find_box_with_token(tx_0.outputs.as_vec(), &reward_token_budget.token_id).unwrap();
    let box_with_vlq_tokens =
        find_box_with_token(signed_mint_vlq_tokens_tx.outputs.as_vec(), &vlq_tokens.token_id).unwrap();
    let box_with_tmp_tokens =
        find_box_with_token(signed_mint_tmp_tokens_tx.outputs.as_vec(), &tmp_tokens.token_id).unwrap();

    let box_with_remaining_funds = signed_mint_tmp_tokens_tx.outputs.get(2).unwrap().clone();

    let target_balance = calc_target_balance(num_transactions_left)?;
    println!("TX_4:");
    println!("  Target balance: {:?}", target_balance);
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

    println!("  Box selection:");
    for ergo_box in &box_selection.boxes {
        println!(
            "   {}: value: {:?}, #tokens: {}",
            ergo_box.box_id(),
            ergo_box.value,
            ergo_box.tokens.as_ref().map(|t| t.len()).unwrap_or(0)
        );
    }

    let input_funds_total = box_selection.boxes.iter().fold(NanoErg::from(0), |acc, ergobox| {
        acc + NanoErg::from(ergobox.value)
    });

    let inputs = TxIoVec::from_vec(
        box_selection
            .boxes
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

    let miner_output = MinerOutput {
        erg_value: NanoErg::from(tx_fee),
    };

    let accumulated_cost = *erg_value_per_box.as_u64() + tx_fee.as_u64();
    let funds_remaining = input_funds_total.safe_sub(NanoErg::from(accumulated_cost));
    let funds_remaining_out_box =
        ErgoBoxCandidateBuilder::new(BoxValue::from(funds_remaining), addr.script()?, height).build()?;
    let output_candidates = vec![
        pool_init_box,
        miner_output.into_candidate(height),
        funds_remaining_out_box,
    ];

    let output_candidates = TxIoVec::from_vec(output_candidates).unwrap();
    let tx_candidate = TransactionCandidate {
        inputs,
        data_inputs: None,
        output_candidates,
    };
    let pool_input_tx = prover.sign(tx_candidate)?;

    println!("  outputs:");
    let mut output_total = 0;
    for ergo_box in &pool_input_tx.outputs {
        println!(
            "    {}: value: {:?}, #tokens: {}",
            ergo_box.box_id(),
            ergo_box.value,
            ergo_box.tokens.as_ref().map(|t| t.len()).unwrap_or(0)
        );
        output_total += ergo_box.value.as_u64();
    }

    assert_eq!(input_funds_total, NanoErg::from(output_total));
    num_transactions_left -= 1;

    // TX 5: Create first LM pool, stake bundle and redeemer out boxes -----------------------------
    let inputs = filter_tx_outputs(pool_input_tx.outputs.clone().to_vec());
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

    println!("TX_5:");
    println!("  Target balance: {:?}", target_balance);
    println!("  Box selection:");
    for ergo_box in &box_selection.boxes {
        println!(
            "   {}: value: {:?}, #tokens: {}",
            ergo_box.box_id(),
            ergo_box.value,
            ergo_box.tokens.as_ref().map(|t| t.len()).unwrap_or(0)
        );
    }
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
        budget_rem: TypedAssetAmount::new(reward_token_budget.token_id, *reward_token_budget.amount.as_u64()),
        reserves_lq: TypedAssetAmount::new(
            initial_lq_token_deposit.token_id,
            initial_lq_token_deposit.amount,
        ),
        reserves_vlq: TypedAssetAmount::new(vlq_tokens.token_id, vlq_token_amount),
        reserves_tmp: TypedAssetAmount::new(tmp_tokens.token_id, tmp_token_amount),
        epoch_ix: None,
        conf,
        erg_value: erg_value_per_box.into(),
    };

    let pool_id = pool.pool_id;
    let lm_pool_box_candidate = pool.into_candidate(height);

    let bundle_key_id: TokenId = inputs.first().0.box_id().into();

    let deposit_output = DepositOutput {
        bundle_key: TypedAssetAmount::new(bundle_key_id, BUNDLE_KEY_AMOUNT_USER),
        redeemer_prop: redeemer_prop.clone(),
        erg_value: erg_value_per_box.into(),
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
        erg_value: erg_value_per_box.into(),
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

    let input_funds_total = box_selection.boxes.iter().fold(NanoErg::from(0), |acc, ergobox| {
        acc + NanoErg::from(ergobox.value)
    });
    let accumulated_cost = NanoErg::from(3 * erg_value_per_box.as_u64() + tx_fee.as_u64());
    let remaining_funds = input_funds_total - accumulated_cost;
    if remaining_funds >= MIN_SAFE_BOX_VALUE {
        let box_of_consumed_tokens =
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

    println!("  outputs:");
    let mut output_total = 0;
    for ergo_box in &init_pool_tx.outputs {
        println!(
            "    {}: value: {:?}, #tokens: {}",
            ergo_box.box_id(),
            ergo_box.value,
            ergo_box.tokens.as_ref().map(|t| t.len()).unwrap_or(0)
        );
        output_total += ergo_box.value.as_u64();
    }

    assert_eq!(input_funds_total, NanoErg::from(output_total));

    Ok((
        vec![
            (tx_0, String::from("Move LQ and reward tokens to single box")),
            (signed_mint_pool_nft_tx, String::from("Mint Pool NFT")),
            (signed_mint_vlq_tokens_tx, String::from("Mint VLQ tokens")),
            (signed_mint_tmp_tokens_tx, String::from("Mint TMP tokens")),
            (pool_input_tx, String::from("Initialise pool-input box")),
            (
                init_pool_tx,
                String::from("Create first LM pool, staking bundle and redeemer out boxes"),
            ),
        ],
        pool_id,
    ))
}

fn find_box_with_token(boxes: &[ErgoBox], token_id: &TokenId) -> Option<ErgoBox> {
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

fn generate_new_wallet() {
    let mnemonic = generate_wallet::generate_bip39_mnemonic();
    println!("Mnemonic seed phrase(KEEP IT SAFE!): {}", mnemonic);
    let (_prover, funding_addr) = Wallet::try_from_seed(SeedPhrase::from(mnemonic)).expect("Invalid seed");

    println!(
        "Wallet's address: {}",
        AddressEncoder::encode_address_as_string(NetworkPrefix::Mainnet, &funding_addr)
    );
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

/// Returns true iff the UTXOs of the given wallet contain all necessary
fn check_utxos(utxos: &Vec<ErgoBox>, config: &DeployPoolInputs) -> Result<(), UtxoError> {
    let mut reward_tokens_count = 0_u64;
    let mut lq_tokens_count = 0_u64;
    let mut other_tokens = vec![];

    let expected_reward_token_quantity = config.conf.program_budget.amount;
    for ergobox in utxos {
        if let Some(ref tokens) = ergobox.tokens {
            for token in tokens {
                if token.token_id == config.conf.program_budget.token_id {
                    reward_tokens_count += *token.amount.as_u64();
                } else if token.token_id == config.initial_lq_token_deposit.token_id {
                    lq_tokens_count += *token.amount.as_u64();
                } else {
                    other_tokens.push(token.clone());
                }
            }
        }
    }
    if !other_tokens.is_empty() {
        return Err(UtxoError::OtherTokensInUtxos(other_tokens));
    }

    if lq_tokens_count < 100 {
        return Err(UtxoError::InsufficientLqTokens);
    }
    if reward_tokens_count < config.conf.program_budget.amount {
        return Err(UtxoError::InsufficientRewardTokens {
            expected_quantity: expected_reward_token_quantity,
            actual_quantity: reward_tokens_count,
        });
    }
    Ok(())
}

fn validate_pool(input: &DeployPoolInputs, current_height: u32) -> Result<(), PoolValidationError> {
    if input.conf.program_start < current_height + 100 {
        return Err(PoolValidationError::StartTooEarly);
    }

    if input.conf.redeem_blocks_delta < input.conf.epoch_len {
        return Err(PoolValidationError::RedeemBlocksDeltaTooLong);
    }

    if input.initial_lq_token_deposit.amount < 100 {
        return Err(PoolValidationError::InsufficientLqTokens);
    }

    let epoch_num = input.conf.epoch_num;
    let max_rounding_error = input.conf.max_rounding_error;
    let budget_amt = input.conf.program_budget.amount;

    if (epoch_num as u64) >= max_rounding_error
        || max_rounding_error * (epoch_num as u64) >= budget_amt / (epoch_num as u64)
    {
        return Err(PoolValidationError::FailedMaxRoundingErrorBounds);
    }
    Ok(())
}

#[derive(Debug)]
pub enum PoolValidationError {
    StartTooEarly,
    RedeemBlocksDeltaTooLong,
    FailedMaxRoundingErrorBounds,
    InsufficientLqTokens,
}

#[derive(Parser)]
#[command(name = "spectrum-deploy-lm-pool")]
#[command(author = "Timothy Ling (@kettlebell) for Spectrum Finance")]
#[command(version = "0.1")]
#[command(about = "Spectrum Finance Liquidity Mining LM pool deployment tool", long_about = None)]
struct AppArgs {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, Subcommand)]
enum Command {
    DeployPool {
        #[arg(long, short)]
        /// Path to the YAML configuration file.
        config_path: String,
    },
    GenerateNewWallet,
}
