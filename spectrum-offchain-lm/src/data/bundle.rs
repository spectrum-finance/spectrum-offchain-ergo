use ergo_lib::ergo_chain_types::Digest32;
use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
use ergo_lib::ergotree_ir::chain::ergo_box::{
    BoxTokens, ErgoBox, ErgoBoxCandidate, NonMandatoryRegisterId, NonMandatoryRegisters, RegisterValue,
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
    pub tmp: TypedAssetAmount<Tmp>,
    pub redeemer_prop: SigmaProp,
    pub erg_value: NanoErg,
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
        }
    }
}

impl IntoBoxCandidate for StakingBundleProto {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        let bundle_key = Token {
            token_id: self.bundle_key_id.token_id,
            amount: TokenAmount::try_from(BUNDLE_KEY_AMOUNT).unwrap(),
        };
        let tokens = BoxTokens::from_vec(if let Ok(tmp) = Token::try_from(self.tmp) {
            vec![Token::try_from(self.vlq).unwrap(), tmp, bundle_key]
        } else {
            vec![Token::try_from(self.vlq).unwrap(), bundle_key]
        })
        .unwrap();
        let registers = NonMandatoryRegisters::try_from(vec![
            RegisterValue::Parsed(Constant::from(self.redeemer_prop)),
            RegisterValue::Parsed(Constant::from(
                TokenId::from(self.pool_id).sigma_serialize_bytes().unwrap(),
            )),
        ])
        .unwrap();
        ErgoBoxCandidate {
            value: BoxValue::from(self.erg_value),
            ergo_tree: BUNDLE_VALIDATOR.clone(),
            tokens: Some(tokens),
            additional_registers: registers,
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
    pub tmp: TypedAssetAmount<Tmp>,
    pub redeemer_prop: SigmaProp,
    pub erg_value: NanoErg,
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
struct StakingBundleWithErgoTreeBytes {
    bundle_key_id: TypedAsset<BundleKey>,
    state_id: BundleStateId,
    pool_id: PoolId,
    vlq: TypedAssetAmount<VirtLq>,
    tmp: TypedAssetAmount<Tmp>,
    /// Sigma-serialized byte representation of `ErgoTree`
    redeemer_prop_bytes: Vec<u8>,
    erg_value: NanoErg,
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
            if tokens.len() == 3 && bx.ergo_tree == *BUNDLE_VALIDATOR {
                let redeemer_prop = bx
                    .get_register(NonMandatoryRegisterId::R4.into())?
                    .v
                    .try_extract_into::<SigmaProp>()
                    .ok()?;
                let pool_id = TokenId::from(
                    Digest32::try_from(
                        bx.get_register(NonMandatoryRegisterId::R5.into())?
                            .v
                            .try_extract_into::<Vec<u8>>()
                            .ok()?,
                    )
                    .ok()?,
                );
                let vlq = tokens.get(0)?.clone();
                let tmp = tokens.get(1)?.clone();
                let bundle_key = tokens.get(2)?.clone();
                return Some(StakingBundle {
                    bundle_key_id: TypedAsset::new(bundle_key.token_id),
                    state_id: BundleStateId::from(bx.box_id()),
                    pool_id: PoolId::from(pool_id),
                    vlq: TypedAssetAmount::from_token(vlq),
                    tmp: TypedAssetAmount::from_token(tmp),
                    redeemer_prop,
                    erg_value: NanoErg::from(bx.value),
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
        Self {
            lower_epoch_ix: conf.epoch_num - (bundle.tmp.amount / bundle.vlq.amount) as u32 + 1,
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
        "boxId": "530f1fdd58e057826fc90c81899ba004a0d2b67c5745e6571587696bd0ac467c",
        "value": 1250000,
        "ergoTree": "19c0062904000400040204020404040404060406040804080404040204000400040204020601010400040a050005000404040204020e20a20a53f905f41ebdd71c2c239f270392d0ae0f23f6bd9f3687d166eea745bbf60400040205000402040204060500050005feffffffffffffffff010502050005000402050005000100d820d601b2a5730000d602db63087201d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b27203730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb27202730800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d6148c720902d615b27205730a00d6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205730f00d61b7e721a05d61c9d7206721bd61d998c720c028c720d02d61e8c721002d61f998c720f02721ed6207310d1ededededed93b272027311007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a70193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b172027312959172137313d802d6219c721399721ba273147e721905d622b2a5731500ededed929a997206721472079c7e9995907219721a72199a721a7316731705721c937213f0721d937221f0721fedededed93cbc272227318938602720e7213b2db6308722273190093860272117221b2db63087222731a00e6c67222040893e4c67222050e8c720401958f7213731bededec929a997206721472079c7e9995907219721a72199a721a731c731d05721c92a39a9a72159c721a7217b27205731e0093721df0721392721f95917219721a731f9c721d99721ba273207e721905d804d621e4c672010704d62299721a7221d6237e722205d62499997321721e9c9972127322722395ed917224732391721f7324edededed9072219972197325909972149c7223721c9a721c7207907ef0998c7208027214069a9d9c99997e7214069d9c7e7206067e7222067e721a0672207e721f067e7224067220937213732693721d73277328",
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
        "boxId": "933eb315cbc33a308312cda4bb41cb0d5804537806d1384eb9c07dbd26886814",
        "value": 1250000,
        "ergoTree": "198f041d04000400040404040400040206010104080400050004020402040204000404050204040400040805feffffffffffffffff01050205000404040004060404040205fcffffffffffffffff010100d80dd601b2a5730000d602db63087201d603e4c6a7050ed604b2a4730100d605db63087204d6068cb2720573020002d607998cb27202730300027206d608e4c6a70408d609db6308a7d60ab27209730400d60bb27205730500d60c7306d60d8cb2720573070002d1ed938cb27202730800017203959372077309d80bd60eb2a5e4e3000400d60fb2a5e4e3010400d610db6308720fd611b27210730a00d612b27209730b00d6138c721202d614b2e4c672040410730c00d615c672010804d61699721495e67215e47215e4c672010704d6177e721605d618b2db6308720e730d00eded93c2720ed07208edededededed93e4c6720f0408720893e4c6720f050e720393c2720fc2a79386028cb27209730e0001730fb27210731000938c7211018c721201939972138c7211029972139c72178c720a0293b27210731100720aed938c7218018c720b01927e8c7218020699999d9c99997e8c720b02069d9c7ee4c672040505067e7216067e721406720c7e998cb2720273120002720d067e99997313720d9c9972067314721706720c720c958f7207731593b2db6308b2a473160073170086029593b1720973188cb27209731900018cb27209731a0001731b731c",
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
            "R4": "08cd03e02fa2bbd85e9298aa37fe2634602a0fba746234fe2a67f04d14deda55fac491",
            "R5": "0e207956620de75192984d639cab2c989269d9a5310ad870ad547426952a9e660699"
        },
        "transactionId": "b5038999043e6ecd617a0a292976fe339d0e4d9ec85296f13610be0c7b16752e",
        "index": 2
    }"#;
}
