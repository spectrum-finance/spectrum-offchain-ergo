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
        let tokens = BoxTokens::from_vec(vec![
            Token::from(self.vlq),
            Token::from(self.tmp),
            Token {
                token_id: self.bundle_key_id.token_id,
                amount: TokenAmount::try_from(BUNDLE_KEY_AMOUNT).unwrap(),
            },
        ])
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
        "boxId": "02f3a00879812244911a4e3075470d605d100bb02c13d7f4152083a6c8f096ae",
        "value": 1250000,
        "ergoTree": "19ec052404000400040204020404040404060406040804080404040204000400040204020400040a050005000404040204020e2074aeba0675c10c7fff46d3aa5e5a8efc55f0b0d87393dcb2f4b0a04be213cecb040004020500040204020406050005000402050205000500d81ed601b2a5730000d602db63087201d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b27203730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb27202730800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d6148c720902d615b27205730a00d6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205730f00d61b7e721a05d61c9d7206721bd61d998c720c028c720d02d61e998c720f028c721002d1ededededed93b272027310007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a70193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b172027311959172137312d802d61f9c721399721ba273137e721905d620b2a5731400ededed929a997206721472079c7e9995907219721a72199a721a7315731605721c937213f0721d93721ff0721eedededed93cbc272207317938602720e7213b2db630872207318009386027211721fb2db63087220731900e6c67220040893e4c67220050e8c720401958f7213731aededec929a997206721472079c7e9995907219721a72199a721a731b731c05721c92a39a9a72159c721a7217b27205731d0093721df0721392721e95917219721a731e9c721d99721ba2731f7e721905d801d61fe4c672010704edededed90721f9972197320909972149c7e99721a721f05721c9a721c7207907ef0998c7208027214069d9c7e721c067e721e067e997212732106937213732293721d7323",
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
        "boxId": "d43f9cd556853127181b2aa5f2bedc1c1e4719b5237caa53d1d6f851e988116b",
        "value": 1250000,
        "ergoTree": "19b803160400040004040404040404020400040005000402040204000502040404000402050205000404040005fcffffffffffffffff010100d80ed601b2a5730000d602db63087201d603e4c6a7050ed604b2a4730100d605db63087204d6068cb2720573020002d607998cb27202730300027206d608e4c6a70408d609db6308a7d60a8cb2720973040001d60bb27209730500d60cb27209730600d60d8c720c02d60e8c720b02d1ed938cb27202730700017203959372077308d808d60fb2a5e4e3000400d610b2a5e4e3010400d611db63087210d612b27211730900d613b2e4c672040410730a00d614c672010804d6157e99721395e67214e47214e4c67201070405d616b2db6308720f730b00eded93c2720fd07208edededededed93e4c672100408720893e4c67210050e720393c27210c2a7938602720a730cb27211730d00938c7212018c720b019399720e8c72120299720e9c7215720d93b27211730e00720ced938c7216018cb27205730f0001927e8c721602069d9c9c7e9de4c6720405057e721305067e720d067e999d720e720d7215067e997206731006958f7207731193b2db6308b2a47312007313008602720a73147315",
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
