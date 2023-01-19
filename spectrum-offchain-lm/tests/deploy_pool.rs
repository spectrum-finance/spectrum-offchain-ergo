use derive_more::From;
use ergo_lib::chain::contract::Contract;
use ergo_lib::chain::ergo_box::box_builder::{ErgoBoxCandidateBuilder, ErgoBoxCandidateBuilderError};
use ergo_lib::chain::ergo_state_context::ErgoStateContext;
use ergo_lib::chain::transaction::{Transaction, TxId, TxIoVec};
use ergo_lib::ergotree_interpreter::sigma_protocol::private_input::{DlogProverInput, PrivateInput};
use ergo_lib::ergotree_interpreter::sigma_protocol::prover::ContextExtension;
use ergo_lib::ergotree_ir::chain::address::{Address, AddressEncoder, NetworkPrefix};
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::{BoxValue, BoxValueError};
use ergo_lib::ergotree_ir::chain::ergo_box::{
    BoxTokens, ErgoBox, ErgoBoxCandidate, NonMandatoryRegisterId, NonMandatoryRegisters,
};
use ergo_lib::ergotree_ir::chain::token::{Token, TokenAmount, TokenId};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::serialization::{SigmaParsingError, SigmaSerializable};
use ergo_lib::wallet::box_selector::{BoxSelector, BoxSelectorError, SimpleBoxSelector};
use ergo_lib::wallet::miner_fee::{MINERS_FEE_ADDRESS, MINERS_FEE_BASE16_BYTES};
use ergo_lib::wallet::secret_key::SecretKey;
use ergo_lib::wallet::signing::{TransactionContext, TxSigningError};
use ergo_lib::wallet::tx_builder::{TxBuilder, TxBuilderError};
use isahc::{AsyncReadResponseExt, HttpClient};
use serde::{Deserialize, Serialize};
use sigma_test_util::force_any_val;
use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};
use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox};
use spectrum_offchain_lm::data::bundle::{StakingBundle, StakingBundleProto, BUNDLE_KEY_AMOUNT_USER};
use spectrum_offchain_lm::data::redeemer::DepositOutput;
use spectrum_offchain_lm::data::PoolId;
use spectrum_offchain_lm::validators::{REDEEM_TEMPLATE, REDEEM_VALIDATOR, REDEEM_VALIDATOR_BYTES};
use thiserror::Error;

use ergo_chain_sync::client::types::{with_path, Url};
use spectrum_offchain::transaction::TransactionCandidate;
use spectrum_offchain_lm::data::pool::{Pool, ProgramConfig};
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

#[derive(Debug, Error, From)]
enum Error {
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
}

pub struct DeployPoolInput {
    conf: ProgramConfig,
    wallet: WalletSecret,
    tx_fee: BoxValue,
    erg_value_per_box: BoxValue,
    change_address: Address,
    height: u32,
    initial_lq_token_deposit: Token,
    max_miner_fee: u64,
    num_epochs_to_delegate: u64,
}

pub async fn deploy_pool(input: DeployPoolInput, explorer: Explorer) -> Result<Vec<Transaction>, Error> {
    let addr = Address::P2Pk(DlogProverInput::from(input.wallet.clone()).public_image());
    let uxtos = explorer.get_utxos(&addr).await;
    deploy_pool_chain_transaction(uxtos, input)
}

fn deploy_pool_chain_transaction(
    uxtos: Vec<ErgoBox>,
    input: DeployPoolInput,
) -> Result<Vec<Transaction>, Error> {
    let DeployPoolInput {
        conf,
        wallet,
        tx_fee,
        erg_value_per_box,
        change_address,
        height,
        initial_lq_token_deposit,
        max_miner_fee,
        num_epochs_to_delegate,
    } = input;
    let reward_token_budget = Token {
        token_id: conf.program_budget.token_id,
        amount: conf.program_budget.amount.try_into().unwrap(),
    };
    let addr = Address::P2Pk(DlogProverInput::from(wallet.clone()).public_image());
    let guard = addr.script()?;
    // We need to create a chain of 6 transactions:
    //   - TX 0: Move initial deposit of LQ and reward tokens into a single box.
    //   - TX 1 to 3: Minting of pool NFT, vLQ tokens and TMP tokens.
    //   - TX 4: Initialize the pool-input box with tokens and parameters necessary to make the
    //     boxes in the next TX.
    //   - TX 5: Create first LM pool, staking bundle and redeemer out boxes.
    //
    // Now assuming that each created box will hold a value of `erg_value_per_box` the total amount
    // of ERG needed is:
    //   6 * tx_fee + 8 * erg_value_per_box
    //
    // Why 8?
    //  - 1 box each for TX 0 to TX 4.
    //  - 3 boxes for TX 5

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
        let last_tx_amt = tx_fee.checked_add(&erg_value_per_box.checked_mul_u32(3)?)?;
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

        let inputs = TxIoVec::from_vec(
            box_selection
                .boxes
                .clone()
                .into_iter()
                .map(|bx| (bx, ContextExtension::empty()))
                .collect::<Vec<_>>(),
        )
        .unwrap();

        let prover = Wallet::trivial(vec![wallet.clone()]);

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

    let box_selector = SimpleBoxSelector::new();
    let box_selection = box_selector.select(
        uxtos.clone(),
        target_balance,
        &[reward_token_budget.clone(), initial_lq_token_deposit.clone()],
    )?;

    let mut builder = ErgoBoxCandidateBuilder::new(erg_value_per_box, addr.script()?, height);
    builder.add_token(reward_token_budget.clone());
    builder.add_token(initial_lq_token_deposit.clone());
    let lq_and_reward_box_candidate = builder.build()?;

    let remaining_funds = ErgoBoxCandidateBuilder::new(
        calc_target_balance(num_transactions_left - 1)?,
        addr.script()?,
        height,
    )
    .build()?;

    let output_candidates = vec![lq_and_reward_box_candidate, remaining_funds];

    let inputs = TxIoVec::from_vec(
        box_selection
            .boxes
            .clone()
            .into_iter()
            .map(|bx| (bx, ContextExtension::empty()))
            .collect::<Vec<_>>(),
    )
    .unwrap();

    let prover = Wallet::trivial(vec![wallet.clone()]);

    let output_candidates = TxIoVec::from_vec(output_candidates).unwrap();
    let tx_candidate = TransactionCandidate {
        inputs,
        data_inputs: None,
        output_candidates,
    };
    num_transactions_left -= 1;
    let tx_0 = prover.sign(tx_candidate)?;

    // TX 1: Mint pool NFT -------------------------------------------------------------------------
    let inputs = filter_tx_outputs(tx_0.outputs.clone());
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
            initial_lq_token_deposit.clone(),
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
    pool_init_box_builder.add_token(initial_lq_token_deposit.clone());
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

    let output_candidates = vec![pool_init_box, remaining_funds];

    let prover = Wallet::trivial(vec![wallet.clone()]);

    let output_candidates = TxIoVec::from_vec(output_candidates).unwrap();
    let tx_candidate = TransactionCandidate {
        inputs,
        data_inputs: None,
        output_candidates,
    };
    let pool_input_tx = prover.sign(tx_candidate)?;
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
            initial_lq_token_deposit.clone(),
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

    let lq_token_amount = *initial_lq_token_deposit.amount.as_u64();
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
            *initial_lq_token_deposit.amount.as_u64(),
        ),
        reserves_vlq: TypedAssetAmount::new(vlq_tokens.token_id.clone(), vlq_token_amount),
        reserves_tmp: TypedAssetAmount::new(tmp_tokens.token_id.clone(), tmp_token_amount),
        epoch_ix: None,
        conf,
        erg_value: erg_value_per_box.into(),
    };

    let lm_pool_box_candidate = pool.into_candidate(height);

    // Build redeemer out candidate
    let redeemer_prop = REDEEM_VALIDATOR
        .clone()
        .with_constant(2, REDEEM_VALIDATOR_BYTES.as_bytes().to_vec().into())
        .unwrap()
        .with_constant(3, initial_lq_token_deposit.token_id.into())
        .unwrap()
        .with_constant(4, (*initial_lq_token_deposit.amount.as_u64() as i64).into())
        .unwrap()
        .with_constant(6, MINERS_FEE_BASE16_BYTES.as_bytes().to_vec().into())
        .unwrap()
        .with_constant(9, (max_miner_fee as i64).into())
        .unwrap();

    let bundle_key_id: TokenId = inputs.first().0.box_id().into();

    let deposit_output = DepositOutput {
        bundle_key: TypedAssetAmount::new(bundle_key_id, BUNDLE_KEY_AMOUNT_USER),
        redeemer_prop: redeemer_prop.clone(),
        erg_value: erg_value_per_box.into(),
    };

    let deposit_output_candidate = deposit_output.into_candidate(height);

    // Build staking bundle candidate

    let staking_bundle = StakingBundleProto {
        bundle_key_id: TypedAsset::new(bundle_key_id),
        pool_id: PoolId::from(pool_nft.token_id),
        vlq: TypedAssetAmount::new(vlq_tokens.token_id, lq_token_amount),
        tmp: TypedAssetAmount::new(tmp_tokens.token_id, lq_token_amount * num_epochs_to_delegate),
        redeemer_prop,
        erg_value: erg_value_per_box.into(),
    };
    let staking_bundle_candidate = staking_bundle.into_candidate(height);

    let output_candidates = vec![
        lm_pool_box_candidate,
        deposit_output_candidate,
        staking_bundle_candidate,
    ];

    let prover = Wallet::trivial(vec![wallet.clone()]);

    let output_candidates = TxIoVec::from_vec(output_candidates).unwrap();
    let tx_candidate = TransactionCandidate {
        inputs,
        data_inputs: None,
        output_candidates,
    };
    let init_pool_tx = prover.sign(tx_candidate)?;

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

#[test]
fn test_deploy_pool_chain_tx() {
    let ergo_state_context = force_any_val::<ErgoStateContext>();

    let secret_key = SecretKey::random_dlog();
    let address = secret_key.get_address_from_public_image();

    let SecretKey::DlogSecretKey(dpi) = secret_key.clone();
    let wallet = WalletSecret::from(dpi);
    let value = BoxValue::try_from(10000000000000_u64).unwrap();
    let tx_fee = BoxValue::from(DEFAULT_MINER_FEE);
    let erg_value_per_box = BoxValue::from(MIN_SAFE_BOX_VALUE);
    let change_address = force_any_val::<Address>();
    let height = 100;
    let initial_lq_token_deposit = force_any_val::<Token>();
    let reward_tokens = force_any_val::<Token>();

    let input_box = ErgoBox::new(
        value,
        Contract::pay_to_address(&address).unwrap().ergo_tree(),
        Some(BoxTokens::try_from(vec![initial_lq_token_deposit.clone(), reward_tokens.clone()]).unwrap()),
        NonMandatoryRegisters::empty(),
        5,
        force_any_val::<TxId>(),
        0,
    )
    .unwrap();

    let conf = ProgramConfig {
        epoch_len: 10,
        epoch_num: 10,
        program_start: 10,
        redeem_blocks_delta: 10,
        max_rounding_error: 100,
        program_budget: TypedAssetAmount::new(reward_tokens.token_id, *reward_tokens.amount.as_u64()),
    };

    let input = DeployPoolInput {
        conf,
        wallet,
        tx_fee,
        erg_value_per_box,
        change_address,
        height,
        initial_lq_token_deposit,
        max_miner_fee: 2_000_000,
        num_epochs_to_delegate: 100,
    };
    let res = deploy_pool_chain_transaction(vec![input_box], input).unwrap();

    let pool_init_tx = res.last().cloned().unwrap();
    let pool = Pool::try_from_box(pool_init_tx.outputs[0].clone()).unwrap();
    let staking_bundle = StakingBundle::try_from_box(pool_init_tx.outputs[2].clone()).unwrap();
}
