use ergo_lib::ergotree_ir::chain::token::TokenId;
use spectrum_offchain::network::ErgoNetwork;

use crate::data::PoolId;

pub struct TokenDetails {
    pub name: String,
    pub description: String,
}

pub async fn get_token_details<TNetwork>(
    pool_id: PoolId,
    reward_token_id: TokenId,
    lq_token_id: TokenId,
    network: &TNetwork,
) -> TokenDetails
where
    TNetwork: ErgoNetwork,
{
    let pool_id_bytes = <Vec<u8>>::from(TokenId::from(pool_id));
    let pool_id_encoding = base16::encode_lower(&pool_id_bytes);

    let reward_token_name = network.get_token_minting_info(reward_token_id).await;
    let liquidity_token_name = network.get_token_minting_info(lq_token_id).await;
    if let (Ok(Some(reward_name)), Ok(Some(lq_name))) = (reward_token_name, liquidity_token_name) {
        TokenDetails{
           name: format!("{}_{}_YF", reward_name.name, lq_name.name),
           description: format!(
                "The representation of your share in the {}/{} (pool id: {}) yield farming pool on the Spectrum Finance platform.",
                reward_name.name,
                lq_name.name,
                pool_id_encoding,
           ),
        }
    } else {
        TokenDetails{
            name: String::from("Spectrum YF staking bundle"), 
            description: format!(
                "The representation of your share in the yield farming pool (pool id: {}) on the Spectrum Finance platform.",
                pool_id_encoding,
            ),
        }
    }
}
