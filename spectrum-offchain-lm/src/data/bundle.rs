use std::collections::HashMap;

use ergo_lib::ergo_chain_types::Digest32;
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::ergo_box::{
    BoxTokens, ErgoBox, ErgoBoxCandidate, NonMandatoryRegisterId, NonMandatoryRegisters,
};
use ergo_lib::ergotree_ir::chain::token::{Token, TokenAmount, TokenId};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::mir::constant::{Constant, TryExtractInto};
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::{ProveDlog, SigmaProp};
use serde::{Deserialize, Serialize};

use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::domain::{TypedAsset, TypedAssetAmount};
use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox};

use crate::data::assets::{BundleKey, Tmp, VirtLq};
use crate::data::pool::{ProgramConfig, INIT_EPOCH_IX};
use crate::data::{BundleId, BundleStateId, PoolId};
use crate::ergo::{NanoErg, MAX_VALUE};
use crate::validators::BUNDLE_VALIDATOR;

/// Prototype of StakeingBundle which guards virtual liquidity and temporal tokens.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StakingBundleProto {
    pub bundle_key_id: TypedAsset<BundleKey>,
    pub pool_id: PoolId,
    pub vlq: TypedAssetAmount<VirtLq>,
    pub tmp: Option<TypedAssetAmount<Tmp>>,
    pub redeemer_prop: SigmaProp,
    pub erg_value: NanoErg,
    pub token_name: String,
    pub token_desc: String,
}

impl StakingBundleProto {
    pub fn finalize(self, state_id: BundleStateId) -> StakingBundle {
        StakingBundle {
            bundle_key_id: self.bundle_key_id,
            state_id,
            pool_id: self.pool_id,
            vlq: self.vlq,
            tmp: self.tmp,
            redeemer_prop: self.redeemer_prop,
            erg_value: self.erg_value,
            token_name: self.token_name,
            token_desc: self.token_desc,
        }
    }
}

impl IntoBoxCandidate for StakingBundleProto {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        let bundle_key = Token {
            token_id: self.bundle_key_id.token_id,
            amount: TokenAmount::try_from(BUNDLE_KEY_AMOUNT).unwrap(),
        };
        let tokens = BoxTokens::from_vec(if let Some(tmp) = self.tmp {
            vec![
                Token::try_from(self.vlq).unwrap(),
                Token::try_from(tmp).unwrap(),
                bundle_key,
            ]
        } else {
            vec![Token::try_from(self.vlq).unwrap(), bundle_key]
        })
        .unwrap();
        let mut registers = HashMap::new();

        registers.insert(
            NonMandatoryRegisterId::R4,
            Constant::from(self.token_name.as_bytes().to_vec()),
        );
        registers.insert(
            NonMandatoryRegisterId::R5,
            Constant::from(self.token_desc.as_bytes().to_vec()),
        );
        registers.insert(NonMandatoryRegisterId::R6, Constant::from(self.redeemer_prop));
        registers.insert(
            NonMandatoryRegisterId::R7,
            Constant::from(TokenId::from(self.pool_id).sigma_serialize_bytes().unwrap()),
        );
        let additional_registers = NonMandatoryRegisters::new(registers).unwrap();
        ErgoBoxCandidate {
            value: BoxValue::from(self.erg_value),
            ergo_tree: BUNDLE_VALIDATOR.clone(),
            tokens: Some(tokens),
            additional_registers,
            creation_height: height,
        }
    }
}

pub const BUNDLE_KEY_AMOUNT: u64 = 1;
pub const BUNDLE_KEY_AMOUNT_USER: u64 = MAX_VALUE - BUNDLE_KEY_AMOUNT;

/// Guards virtual liquidity and temporal tokens.
/// Staking Bundle is a persistent, self-reproducible, on-chain entity.
#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
#[serde(from = "StakingBundleWithErgoTreeBytes")]
#[serde(into = "StakingBundleWithErgoTreeBytes")]
pub struct StakingBundle {
    pub bundle_key_id: TypedAsset<BundleKey>,
    pub state_id: BundleStateId,
    pub pool_id: PoolId,
    pub vlq: TypedAssetAmount<VirtLq>,
    pub tmp: Option<TypedAssetAmount<Tmp>>,
    pub redeemer_prop: SigmaProp,
    pub erg_value: NanoErg,
    pub token_name: String,
    pub token_desc: String,
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
struct StakingBundleWithErgoTreeBytes {
    bundle_key_id: TypedAsset<BundleKey>,
    state_id: BundleStateId,
    pool_id: PoolId,
    vlq: TypedAssetAmount<VirtLq>,
    tmp: Option<TypedAssetAmount<Tmp>>,
    /// Sigma-serialized byte representation of `ErgoTree`
    redeemer_prop_bytes: Vec<u8>,
    erg_value: NanoErg,
    token_name: String,
    token_desc: String,
}

impl From<StakingBundleWithErgoTreeBytes> for StakingBundle {
    fn from(s: StakingBundleWithErgoTreeBytes) -> Self {
        Self {
            bundle_key_id: s.bundle_key_id,
            state_id: s.state_id,
            pool_id: s.pool_id,
            vlq: s.vlq,
            tmp: s.tmp,
            redeemer_prop: SigmaProp::from(
                ProveDlog::try_from(ErgoTree::sigma_parse_bytes(&s.redeemer_prop_bytes).unwrap()).unwrap(),
            ),
            erg_value: s.erg_value,
            token_name: s.token_name,
            token_desc: s.token_desc,
        }
    }
}

impl From<StakingBundle> for StakingBundleWithErgoTreeBytes {
    fn from(s: StakingBundle) -> Self {
        Self {
            bundle_key_id: s.bundle_key_id,
            state_id: s.state_id,
            pool_id: s.pool_id,
            vlq: s.vlq,
            tmp: s.tmp,
            redeemer_prop_bytes: s.redeemer_prop.prop_bytes().unwrap(),
            erg_value: s.erg_value,
            token_name: s.token_name,
            token_desc: s.token_desc,
        }
    }
}

impl StakingBundle {
    pub fn from_proto(p: StakingBundleProto, state_id: BundleStateId) -> Self {
        Self {
            bundle_key_id: p.bundle_key_id,
            state_id,
            pool_id: p.pool_id,
            vlq: p.vlq,
            tmp: p.tmp,
            redeemer_prop: p.redeemer_prop,
            erg_value: p.erg_value,
            token_name: p.token_name,
            token_desc: p.token_desc,
        }
    }

    pub fn bundle_id(&self) -> BundleId {
        BundleId::from(self.bundle_key_id.token_id)
    }
}

impl From<StakingBundle> for StakingBundleProto {
    fn from(sb: StakingBundle) -> Self {
        Self {
            bundle_key_id: sb.bundle_key_id,
            pool_id: sb.pool_id,
            vlq: sb.vlq,
            tmp: sb.tmp,
            redeemer_prop: sb.redeemer_prop,
            erg_value: sb.erg_value,
            token_name: sb.token_name,
            token_desc: sb.token_desc,
        }
    }
}

impl OnChainEntity for StakingBundle {
    type TEntityId = BundleId;
    type TStateId = BundleStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.bundle_id()
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.state_id
    }
}

impl IntoBoxCandidate for StakingBundle {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        StakingBundleProto::from(self).into_candidate(height)
    }
}

impl TryFromBox for StakingBundle {
    fn try_from_box(bx: ErgoBox) -> Option<StakingBundle> {
        if let Some(ref tokens) = bx.tokens {
            // NOTE: the staking bundle normally contains 3 tokens, but after the final compounding
            // all the TMP tokens of the bundle are consumed. The resulting box representing the
            // staking bundle doesn't actually conform to the requirements of its contract since
            // it will only contain the VLQ tokens and the bundle id. It's fine though since
            // only the Redeem order will ever interact with this box.
            if (tokens.len() == 3 || tokens.len() == 2) && bx.ergo_tree == *BUNDLE_VALIDATOR {
                let redeemer_prop = bx
                    .get_register(NonMandatoryRegisterId::R6.into())?
                    .v
                    .try_extract_into::<SigmaProp>()
                    .ok()?;
                let pool_id = TokenId::from(
                    Digest32::try_from(
                        bx.get_register(NonMandatoryRegisterId::R7.into())?
                            .v
                            .try_extract_into::<Vec<u8>>()
                            .ok()?,
                    )
                    .ok()?,
                );
                let token_name = bx
                    .get_register(NonMandatoryRegisterId::R4.into())?
                    .v
                    .try_extract_into::<Vec<u8>>()
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())?;
                let token_desc = bx
                    .get_register(NonMandatoryRegisterId::R5.into())?
                    .v
                    .try_extract_into::<Vec<u8>>()
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())?;
                let vlq = tokens.get(0)?.clone();
                let (tmp, bundle_key) = if tokens.len() == 3 {
                    let tmp = Some(TypedAssetAmount::from_token(tokens.get(1)?.clone()));
                    let bundle_key = tokens.get(2)?.clone();
                    (tmp, bundle_key)
                } else {
                    let bundle_key = tokens.get(1)?.clone();

                    // No TMP tokens.
                    let tmp = None;
                    (tmp, bundle_key)
                };
                return Some(StakingBundle {
                    bundle_key_id: TypedAsset::new(bundle_key.token_id),
                    state_id: BundleStateId::from(bx.box_id()),
                    pool_id: PoolId::from(pool_id),
                    vlq: TypedAssetAmount::from_token(vlq),
                    tmp,
                    redeemer_prop,
                    erg_value: NanoErg::from(bx.value),
                    token_name,
                    token_desc,
                });
            }
        }
        None
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct IndexedBundle<B> {
    pub bundle: B,
    pub lower_epoch_ix: u32,
}

impl IndexedBundle<StakingBundle> {
    pub fn new(bundle: StakingBundle, conf: ProgramConfig) -> Self {
        let tmp_amount = if let Some(t) = bundle.tmp { t.amount } else { 0 };
        Self {
            lower_epoch_ix: conf.epoch_num - (tmp_amount / bundle.vlq.amount) as u32 + 1,
            bundle,
        }
    }

    pub fn init(bundle: StakingBundle) -> Self {
        Self {
            bundle,
            lower_epoch_ix: INIT_EPOCH_IX,
        }
    }
}

pub type IndexedStakingBundle = IndexedBundle<StakingBundle>;

impl<T> OnChainEntity for IndexedBundle<T>
where
    T: OnChainEntity,
{
    type TEntityId = T::TEntityId;
    type TStateId = T::TStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.bundle.get_self_ref()
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.bundle.get_self_state_ref()
    }
}

#[cfg(test)]
mod tests {
    use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;

    use spectrum_offchain::event_sink::handlers::types::TryFromBox;

    use crate::data::bundle::{IndexedBundle, StakingBundle};
    use crate::data::pool::Pool;

    #[test]
    fn bundle_compatible_with_pool() {
        let pool_bx: ErgoBox = serde_json::from_str(POOL_JSON).unwrap();
        let pool = Pool::try_from_box(pool_bx).unwrap();
        let bundle_bx: ErgoBox = serde_json::from_str(BUNDLE_JSON).unwrap();
        let bundle = StakingBundle::try_from_box(bundle_bx).unwrap();
        let indexed_bundle = IndexedBundle::new(bundle, pool.conf);
        println!("IB: {:?}", indexed_bundle);
        println!("P: {:?}", pool);
        println!("P: {:?}", pool.epochs_left_to_process());
    }

    const POOL_JSON: &str = r#"{
        "boxId": "79c4cb46e5b862816028e694366e7567bdc398e58c77a52b1929c86b1ea9a69d",
        "value": 1250000,
        "ergoTree": "19c0062904000400040204020404040404060406040804080404040204000400040204020601010400040a050005000404040204020e202045638fde5b28db0f08d3ebe28663bc21333348cd7679e11500931a7f9070900400040205000402040204060500050005feffffffffffffffff010502050005000402050005000100d820d601b2a5730000d602db63087201d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b27203730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb27202730800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d6148c720902d615b27205730a00d6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205730f00d61b7e721a05d61c9d7206721bd61d998c720c028c720d02d61e8c721002d61f998c720f02721ed6207310d1ededededed93b272027311007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a70193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b172027312959172137313d802d6219c721399721ba273147e721905d622b2a5731500ededed929a997206721472079c7e9995907219721a72199a721a7316731705721c937213f0721d937221f0721fedededed93cbc272227318938602720e7213b2db6308722273190093860272117221b2db63087222731a00e6c67222060893e4c67222070e8c720401958f7213731bededec929a997206721472079c7e9995907219721a72199a721a731c731d05721c92a39a9a72159c721a7217b27205731e0093721df0721392721f95917219721a731f9c721d99721ba273207e721905d804d621e4c672010704d62299721a7221d6237e722205d62499997321721e9c9972127322722395ed917224732391721f7324edededed9072219972197325909972149c7223721c9a721c7207907ef0998c7208027214069a9d9c99997e7214069d9c7e7206067e7222067e721a0672207e721f067e7224067220937213732693721d73277328",
        "assets": [
            {
                "tokenId": "7956620de75192984d639cab2c989269d9a5310ad870ad547426952a9e660699",
                "amount": 1
            },
            {
                "tokenId": "0779ec04f2fae64e87418a1ad917639d4668f78484f45df962b0dec14a2591d2",
                "amount": 299993
            },
            {
                "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                "amount": 283146
            },
            {
                "tokenId": "3fdce3da8d364f13bca60998c20660c79c19923f44e141df01349d2e63651e86",
                "amount": 99716855
            },
            {
                "tokenId": "c256908dd9fd477bde350be6a41c0884713a1b1d589357ae731763455ef28c10",
                "amount": 1496035970
            }
        ],
        "creationHeight": 923467,
        "additionalRegisters": {
            "R4": "100490031eeeca70c801",
            "R5": "05becf24",
            "R6": "05d00f",
            "R7": "0402"
        },
        "transactionId": "b5038999043e6ecd617a0a292976fe339d0e4d9ec85296f13610be0c7b16752e",
        "index": 0
    }"#;

    const BUNDLE_JSON: &str = r#"{
        "boxId": "c2c4af43342777f027e5a49bb3c3410c577864e1b89687c8864b4a6096ba274c",
        "value": 1250000,
        "ergoTree": "19c404220400040004040404040004020601010601000400050004020404040205feffffffffffffffff0104080502040004020502040405020402040004000101010005000404040004060404040205fcffffffffffffffff010100d80dd601b2a5730000d602db63087201d603e4c6a7070ed604b2a4730100d605db63087204d6068cb2720573020002d607998cb27202730300027206d608e4c6a70608d609db6308a7d60ab27209730400d60bb27205730500d60c7306d60d7307d1ed938cb27202730800017203959372077309d80cd60eb2a5e4e3000400d60fb2a5e4e3010400d610b2e4c672040410730a00d611c672010804d61299721095e67211e47211e4c672010704d6138cb27209730b0001d614db6308720fd615b27209730c00d6168c721502d6177e721205d6189972169c72178c720a02d619999d9c99997e8c720b02069d9c7ee4c672040505067e7212067e721006720c7e7218067e9999730d8cb27205730e00029c997206730f721706720ceded93c2720ed07208edededed93e4c6720f0608720893e4c6720f070e720393c2720fc2a7959172127310d801d61ab27214731100eded93860272137312b27214731300938c721a018c721501939972168c721a02721893860272137314b2721473150093b27214731600720a95917219720dd801d61ab2db6308720e731700ed938c721a018c720b01927e8c721a0206997219720c95937219720d73187319958f7207731a93b2db6308b2a4731b00731c0086029593b17209731d8cb27209731e00018cb27209731f000173207321",
        "assets": [
            {
                "tokenId": "3fdce3da8d364f13bca60998c20660c79c19923f44e141df01349d2e63651e86",
                "amount": 100
            },
            {
                "tokenId": "c256908dd9fd477bde350be6a41c0884713a1b1d589357ae731763455ef28c10",
                "amount": 1400
            },
            {
                "tokenId": "251177a50ed3d4df8fc8575b3d9e03a0ba81f506a329a3ba7d8bb20994303794",
                "amount": 1
            }
        ],
        "creationHeight": 923467,
        "additionalRegisters": {
            "R4": "0e00",
            "R5": "0e00",
            "R6": "08cd03e02fa2bbd85e9298aa37fe2634602a0fba746234fe2a67f04d14deda55fac491",
            "R7": "0e207956620de75192984d639cab2c989269d9a5310ad870ad547426952a9e660699"
        },
        "transactionId": "b5038999043e6ecd617a0a292976fe339d0e4d9ec85296f13610be0c7b16752e",
        "index": 2
    }"#;
}
