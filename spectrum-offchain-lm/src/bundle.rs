use std::sync::Arc;

use async_trait::async_trait;
use log::trace;
use tokio::sync::Mutex;

use spectrum_offchain::data::unique_entity::{Confirmed, Predicted, Traced};
use spectrum_offchain::data::OnChainEntity;

use crate::data::bundle::{IndexedStakingBundle, StakingBundle};
use crate::data::{AsBox, BundleId, BundleStateId, PoolId};

pub mod process;
pub mod rocksdb;

#[async_trait(?Send)]
pub trait BundleRepo {
    /// Select bundles corresponding to the pool with given id that are eligible for rewards in
    /// epoch `epoch_ix`.
    async fn select(&self, pool_id: PoolId, epoch_ix: u32) -> Vec<BundleId>;
    /// False-positive analog of `exists()`.
    async fn may_exist(&self, sid: BundleStateId) -> bool;
    /// Get particular state of staking bundle.
    async fn get_state(&self, state_id: BundleStateId) -> Option<AsBox<IndexedStakingBundle>>;
    /// Invalidate bundle state snapshot corresponding to the given `state_id`.
    async fn invalidate(&self, state_id: BundleStateId);
    /// Mark given bundle as permanently eliminated.
    async fn eliminate(&self, bundle: IndexedStakingBundle);
    /// Persist confirmed state staking bundle.
    async fn put_confirmed(&self, bundle: Confirmed<AsBox<IndexedStakingBundle>>);
    /// Persist predicted state staking bundle.
    async fn put_predicted(&self, bundle: Traced<Predicted<AsBox<IndexedStakingBundle>>>);
    /// Get last confirmed staking bundle.
    async fn get_last_confirmed(&self, id: BundleId) -> Option<Confirmed<AsBox<StakingBundle>>>;
    /// Get last predicted staking bundle.
    async fn get_last_predicted(&self, id: BundleId) -> Option<Predicted<AsBox<StakingBundle>>>;
    /// Get state id preceding given predicted state.
    async fn get_prediction_predecessor(&self, id: BundleStateId) -> Option<BundleStateId>;
}

pub struct BundleRepoTracing<R> {
    inner: R,
}

impl<B> BundleRepoTracing<B> {
    pub fn wrap(backlog: B) -> Self {
        Self { inner: backlog }
    }
}

#[async_trait(?Send)]
impl<R> BundleRepo for BundleRepoTracing<R>
where
    R: BundleRepo,
{
    async fn select(&self, pool_id: PoolId, epoch_ix: u32) -> Vec<BundleId> {
        let res = self.inner.select(pool_id, epoch_ix).await;
        trace!(target: "bundles", "select(pool_id: {}, epoch_ix: {}) -> {:?}", pool_id, epoch_ix, res);
        res
    }

    async fn may_exist(&self, sid: BundleStateId) -> bool {
        self.inner.may_exist(sid).await
    }

    async fn get_state(&self, state_id: BundleStateId) -> Option<AsBox<IndexedStakingBundle>> {
        self.inner.get_state(state_id).await
    }

    async fn invalidate(&self, state_id: BundleStateId) {
        trace!(target: "bundles", "invalidate(state_id: {})", state_id);
        self.inner.invalidate(state_id).await
    }

    async fn eliminate(&self, bundle: IndexedStakingBundle) {
        trace!(target: "bundles", "eliminate(bundle: {:?})", bundle.bundle.bundle_id());
        self.inner.eliminate(bundle).await
    }

    async fn put_confirmed(&self, bundle: Confirmed<AsBox<IndexedStakingBundle>>) {
        trace!(target: "bundles", "put_confirmed(bundle: {:?})", bundle.0.1);
        self.inner.put_confirmed(bundle).await
    }

    async fn put_predicted(&self, bundle: Traced<Predicted<AsBox<IndexedStakingBundle>>>) {
        self.inner.put_predicted(bundle).await
    }

    async fn get_last_confirmed(&self, id: BundleId) -> Option<Confirmed<AsBox<StakingBundle>>> {
        self.inner.get_last_confirmed(id).await
    }

    async fn get_last_predicted(&self, id: BundleId) -> Option<Predicted<AsBox<StakingBundle>>> {
        self.inner.get_last_predicted(id).await
    }

    async fn get_prediction_predecessor(&self, id: BundleStateId) -> Option<BundleStateId> {
        self.inner.get_prediction_predecessor(id).await
    }
}

pub async fn resolve_bundle_state<TRepo>(
    bundle_id: BundleId,
    repo: Arc<Mutex<TRepo>>,
) -> Option<AsBox<StakingBundle>>
where
    TRepo: BundleRepo,
{
    let (predicted, confirmed) = {
        let repo_guard = repo.lock().await;
        let predicted = repo_guard.get_last_predicted(bundle_id).await;
        let confirmed = repo_guard.get_last_confirmed(bundle_id).await;
        (predicted, confirmed)
    };
    match (confirmed, predicted) {
        (Some(Confirmed(conf)), Some(Predicted(pred))) => {
            let anchoring_point = conf;
            let anchoring_sid = anchoring_point.1.get_self_state_ref();
            let predicted_sid = pred.1.get_self_state_ref();
            let prediction_is_anchoring_point = predicted_sid == anchoring_sid;
            let prediction_is_valid =
                prediction_is_anchoring_point || is_linking(predicted_sid, anchoring_sid, repo).await;
            let safe_point = if prediction_is_valid {
                pred
            } else {
                anchoring_point
            };
            Some(safe_point)
        }
        (Some(Confirmed(conf)), _) => Some(conf),
        _ => None,
    }
}

#[allow(clippy::await_holding_lock)]
async fn is_linking<TRepo>(sid: BundleStateId, anchoring_sid: BundleStateId, repo: Arc<Mutex<TRepo>>) -> bool
where
    TRepo: BundleRepo,
{
    let mut head_sid = sid;
    let repo = repo.lock().await;
    loop {
        match repo.get_prediction_predecessor(head_sid).await {
            None => return false,
            Some(prev_state_id) => {
                if prev_state_id == anchoring_sid {
                    return true;
                } else {
                    head_sid = prev_state_id;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::{ProveDlog, SigmaProp};
    use ergo_lib::{
        chain::{ergo_box::box_builder::ErgoBoxCandidateBuilder, transaction::TxId},
        ergo_chain_types::Digest32,
        ergotree_ir::{
            chain::{
                ergo_box::{box_value::BoxValue, BoxId, ErgoBox, NonMandatoryRegisterId},
                token::{Token, TokenAmount, TokenId},
            },
            ergo_tree::ErgoTree,
            mir::constant::Constant,
            serialization::SigmaSerializable,
        },
    };
    use rand::RngCore;
    use sigma_test_util::force_any_val;

    use spectrum_offchain::{
        data::{
            unique_entity::{Confirmed, Predicted, Traced},
            OnChainEntity,
        },
        event_sink::handlers::types::TryFromBox,
    };

    use crate::data::bundle::{IndexedBundle, IndexedStakingBundle};
    use crate::data::PoolId;
    use crate::data::{AsBox, BundleStateId};
    use crate::validators::BUNDLE_VALIDATOR;

    use super::{rocksdb::BundleRepoRocksDB, BundleRepo, StakingBundle};

    #[tokio::test]
    async fn test_rocksdb_may_exist() {
        let client = rocks_db_client();
        test_bundle_repo_may_exist(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_predicted() {
        let client = rocks_db_client();
        test_bundle_repo_predicted(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_confirmed() {
        let client = rocks_db_client();
        test_bundle_repo_confirmed(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_invalidate() {
        let client = rocks_db_client();
        test_bundle_repo_invalidate(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_select() {
        let client = rocks_db_client();
        test_bundle_repo_select(client).await;
    }

    async fn test_bundle_repo_may_exist<C: BundleRepo>(client: C) {
        let pool_id_0 = PoolId::from(force_any_val::<TokenId>());
        let bundles = gen_bundles(pool_id_0, 0, 3);
        let box_ids: Vec<BoxId> = bundles.iter().map(|b| b.0.box_id()).collect();
        for bundle in bundles {
            client
                .put_predicted(Traced {
                    state: Predicted(bundle),
                    prev_state_id: None,
                })
                .await;
        }

        for box_id in box_ids {
            assert!(client.may_exist(BundleStateId::from(box_id)).await);
        }
    }

    async fn test_bundle_repo_predicted<C: BundleRepo>(client: C) {
        let pool_id_0 = PoolId::from(force_any_val::<TokenId>());
        let bundles = gen_bundles(pool_id_0, 0, 3);
        let box_ids: Vec<BoxId> = bundles.iter().map(|b| b.0.box_id()).collect();

        for i in 1..bundles.len() {
            let traced = Traced {
                state: Predicted(bundles[i].clone()),
                prev_state_id: Some(BundleStateId::from(bundles[i - 1].0.box_id())),
            };
            client.put_predicted(traced).await;
        }

        for i in 1..bundles.len() {
            let pred = client
                .get_prediction_predecessor(BundleStateId::from(box_ids[i]))
                .await;
            assert_eq!(pred, Some(BundleStateId::from(box_ids[i - 1])));
        }
    }

    async fn test_bundle_repo_confirmed<C: BundleRepo>(client: C) {
        let pool_id_0 = PoolId::from(force_any_val::<TokenId>());
        let bundles = gen_bundles(pool_id_0, 0, 3);

        for b in &bundles {
            client.put_confirmed(Confirmed(b.clone())).await;
        }

        for i_bundle in &bundles {
            let pred = client.get_last_confirmed(i_bundle.1.bundle.bundle_id()).await;
            assert_eq!(pred.unwrap().0 .0, i_bundle.0);
        }
    }

    async fn test_bundle_repo_invalidate<C: BundleRepo>(client: C) {
        let pool_id_0 = PoolId::from(force_any_val::<TokenId>());
        let bundles = gen_bundles(pool_id_0, 0, 3);

        for i in 1..bundles.len() {
            let traced = Traced {
                state: Predicted(bundles[i].clone()),
                prev_state_id: Some(BundleStateId::from(bundles[i - 1].0.box_id())),
            };
            client.put_predicted(traced).await;

            // Invalidate
            client.invalidate(bundles[i].get_self_state_ref()).await;
            let predicted = client.get_last_predicted(bundles[i].get_self_ref()).await;
            assert!(predicted.is_none());
        }
    }

    async fn test_bundle_repo_select<C: BundleRepo>(client: C) {
        let pool_id_0 = PoolId::from(TokenId::from(Digest32::from([0u8; 32])));
        let pool_id_1 = PoolId::from(TokenId::from(Digest32::from([1u8; 32])));

        let bundles_pool_0_epoch_1 = gen_bundles(pool_id_0, 1, 2);
        let bundles_pool_0_epoch_2 = gen_bundles(pool_id_0, 2, 4);
        let bundles_pool_1_epoch_1 = gen_bundles(pool_id_1, 1, 2);
        let bundles_pool_1_epoch_2 = gen_bundles(pool_id_1, 2, 3);

        let all_bundles = bundles_pool_0_epoch_1
            .iter()
            .chain(&bundles_pool_0_epoch_2)
            .chain(&bundles_pool_1_epoch_1)
            .chain(&bundles_pool_1_epoch_2);
        for b in all_bundles {
            client.put_confirmed(Confirmed(b.clone())).await;
        }
        let pool_0_ep_1_res = client.select(pool_id_0, 1).await;
        let pool_0_ep_1_expected = bundles_pool_0_epoch_1.iter();
        assert!(!pool_0_ep_1_res.is_empty());
        for AsBox(_, bun) in pool_0_ep_1_expected {
            assert!(pool_0_ep_1_res
                .iter()
                .find(|bid| bun.bundle.bundle_id() == **bid)
                .is_some());
        }
        let pool_1_ep_2_res = client.select(pool_id_1, 2).await;
        let pool_1_ep_2_expected = bundles_pool_1_epoch_2;
        assert!(!pool_1_ep_2_res.is_empty());
        for AsBox(_, bun) in pool_1_ep_2_expected {
            assert!(pool_1_ep_2_res
                .iter()
                .find(|bid| bun.bundle.bundle_id() == **bid)
                .is_some());
        }
    }

    fn rocks_db_client() -> BundleRepoRocksDB {
        let rnd = rand::thread_rng().next_u32();
        BundleRepoRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
        }
    }

    fn gen_bundles(pool_id: PoolId, epoch_ix: u32, num: usize) -> Vec<AsBox<IndexedStakingBundle>> {
        let mut res = vec![];
        for _ in 0..num {
            let value = force_any_val::<BoxValue>();
            let height = force_any_val::<u32>();
            let ergo_tree = BUNDLE_VALIDATOR.clone();

            let mut builder = ErgoBoxCandidateBuilder::new(value, ergo_tree, height);

            let redeemer_prop = "0008cd03b196b978d77488fba3138876a40a40b9a046c2fbb5ecfa13d4ecf8f1eec52aec";
            let tree = ErgoTree::sigma_parse_bytes(&base16::decode(redeemer_prop).unwrap()).unwrap();
            let sigma_prop = SigmaProp::from(ProveDlog::try_from(tree).unwrap());
            builder.set_register_value(
                NonMandatoryRegisterId::R4,
                Constant::from(String::from("").as_bytes().to_vec()),
            );
            builder.set_register_value(
                NonMandatoryRegisterId::R5,
                Constant::from(String::from("").as_bytes().to_vec()),
            );
            builder.set_register_value(NonMandatoryRegisterId::R6, Constant::from(sigma_prop));
            builder.set_register_value(NonMandatoryRegisterId::R7, Constant::from(TokenId::from(pool_id)));

            let vlq = Token {
                token_id: gen_from_rnd_digest_32::<TokenId>(),
                amount: force_any_val::<TokenAmount>(),
            };
            let tmp = Token {
                token_id: gen_from_rnd_digest_32::<TokenId>(),
                amount: force_any_val::<TokenAmount>(),
            };

            builder.add_token(vlq.clone());
            builder.add_token(tmp.clone());

            let bundle_key = Token {
                token_id: gen_from_rnd_digest_32::<TokenId>(),
                amount: force_any_val::<TokenAmount>(),
            };

            builder.add_token(bundle_key.clone());

            let candidate = builder.build().unwrap();

            let tx_id = force_any_val::<TxId>();
            let eb = ErgoBox::from_box_candidate(&candidate, tx_id, 0).unwrap();

            let staking_bundle = IndexedBundle {
                bundle: StakingBundle::try_from_box(eb.clone()).unwrap(),
                lower_epoch_ix: epoch_ix,
            };
            res.push(AsBox(eb, staking_bundle));
        }
        res
    }

    fn gen_from_rnd_digest_32<T: From<Digest32>>() -> T {
        let mut bytes: Vec<u8> = std::iter::repeat(0_u8).take(32).collect();
        rand::thread_rng().fill_bytes(&mut bytes);
        let d = Digest32::try_from(bytes).unwrap();
        T::from(d)
    }
}
