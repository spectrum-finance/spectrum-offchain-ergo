use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use spectrum_offchain::data::unique_entity::{Confirmed, Predicted, Traced};
use spectrum_offchain::data::OnChainEntity;

use crate::data::bundle::{IndexedStakingBundle, StakingBundle};
use crate::data::{AsBox, BundleId, BundleStateId, PoolId};

pub mod process;
pub mod rocksdb;

#[async_trait]
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

#[allow(clippy::await_holding_lock)]
pub async fn resolve_bundle_state<TRepo>(
    bundle_id: BundleId,
    repo: Arc<Mutex<TRepo>>,
) -> Option<AsBox<StakingBundle>>
where
    TRepo: BundleRepo,
{
    let repo_guard = repo.lock();
    let predicted = repo_guard.get_last_predicted(bundle_id).await;
    let confirmed = repo_guard.get_last_confirmed(bundle_id).await;
    drop(repo_guard);
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
async fn is_linking<TRepo>(
    sid: BundleStateId,
    anchoring_sid: BundleStateId,
    persistence: Arc<Mutex<TRepo>>,
) -> bool
where
    TRepo: BundleRepo,
{
    let mut head_sid = sid;
    loop {
        match persistence.lock().get_prediction_predecessor(head_sid).await {
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
            unique_entity::{Confirmed, Predicted, Traced, Unconfirmed},
            OnChainEntity,
        },
        event_sink::handlers::types::TryFromBox,
    };

    use crate::data::bundle::{IndexedBundle, IndexedStakingBundle};
    use crate::{
        data::{AsBox, BundleStateId},
        validators::bundle_validator,
    };

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
        let epoch_len = 1;
        let bundles = gen_bundles(0, epoch_len);
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
        let epoch_len = 1;
        let bundles = gen_bundles(0, epoch_len);
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
        let epoch_len = 1;
        let i_bundles = gen_bundles(0, epoch_len);

        for b in &i_bundles {
            client.put_confirmed(Confirmed(b.clone())).await;
        }

        for i_bundle in &i_bundles {
            let pred = client.get_last_confirmed(i_bundle.1.bundle.bundle_id()).await;
            assert_eq!(pred.unwrap().0 .0, i_bundle.0);
        }
    }

    async fn test_bundle_repo_invalidate<C: BundleRepo>(client: C) {
        let epoch_len = 1;
        let bundles = gen_bundles(0, epoch_len);

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
        let epoch_len = 1;
        let bundles_epoch_0 = gen_bundles(0, epoch_len);
        let pool_id = bundles_epoch_0[0].1.bundle.pool_id;
        for b in &bundles_epoch_0 {
            client.put_confirmed(Confirmed(b.clone())).await;
        }
        let selected_bundles = client.select(pool_id, 1).await;
        assert_eq!(selected_bundles.len(), bundles_epoch_0.len());
        for b_id in selected_bundles {
            assert!(bundles_epoch_0.iter().any(|b| b.1.bundle.bundle_id() == b_id));
        }
    }

    fn rocks_db_client() -> BundleRepoRocksDB {
        let rnd = rand::thread_rng().next_u32();
        BundleRepoRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
            epoch_len: 10,
        }
    }

    fn gen_bundles(epoch_ix: u32, epoch_len: u32) -> Vec<AsBox<IndexedStakingBundle>> {
        let start_height = epoch_ix * epoch_len;
        let last_height = (epoch_ix + 1) * epoch_len - 1;

        let pool_id = force_any_val::<TokenId>();

        let mut res = vec![];
        for height in start_height..=last_height {
            let value = force_any_val::<BoxValue>();
            let ergo_tree = bundle_validator();

            let mut builder = ErgoBoxCandidateBuilder::new(value, ergo_tree, height);

            let redeemer_prop = force_any_val::<ErgoTree>();
            builder.set_register_value(
                NonMandatoryRegisterId::R4,
                Constant::from(redeemer_prop.sigma_serialize_bytes().unwrap()),
            );

            builder.set_register_value(NonMandatoryRegisterId::R5, Constant::from(pool_id));

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
                init_epoch_ix: 0,
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
