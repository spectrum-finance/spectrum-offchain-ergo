use std::sync::Arc;

use async_trait::async_trait;
use spectrum_offchain::{
    binary::prefixed_key,
    data::{
        unique_entity::{Confirmed, Predicted, Traced, Unconfirmed},
        OnChainEntity,
    },
};
use tokio::task::spawn_blocking;

use crate::data::{AsBox, BundleId, BundleStateId, PoolId};

use super::{BundleRepo, EpochHeightRange, StakingBundle};

pub struct BundleRepoRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
    pub epoch_len: u32,
}

#[async_trait]
impl BundleRepo for BundleRepoRocksDB {
    fn get_epoch_length(&self) -> u32 {
        self.epoch_len
    }

    fn get_height_range(&self, epoch_ix: u32) -> EpochHeightRange {
        let first = (epoch_ix - 1) * self.epoch_len;
        let last = epoch_ix * self.epoch_len - 1;
        EpochHeightRange { first, last }
    }

    async fn select(&self, pool_id: PoolId, epoch_ix: u32) -> Vec<BundleId> {
        assert!(epoch_ix > 0);

        let EpochHeightRange { first, last } = self.get_height_range(epoch_ix);

        let db = self.db.clone();

        spawn_blocking(move || {
            let mut bundle_ids = vec![];
            for (_, value) in db
                .iterator(rocksdb::IteratorMode::From(
                    &bincode::serialize(STATE_PREFIX.as_bytes()).unwrap(),
                    rocksdb::Direction::Forward,
                ))
                .flatten()
            {
                if let Ok(bundle) = bincode::deserialize::<'_, AsBox<StakingBundle>>(&value) {
                    let bundle_height = bundle.0.creation_height;
                    if bundle.1.pool_id == pool_id && bundle_height <= last && bundle_height >= first {
                        bundle_ids.push(bundle.1.bundle_id());
                    }
                }
            }
            bundle_ids
        })
        .await
        .unwrap()
    }

    async fn may_exist(&self, sid: BundleStateId) -> bool {
        let db = self.db.clone();
        let state_key = prefixed_key(STATE_PREFIX, &sid);
        spawn_blocking(move || db.key_may_exist(state_key)).await.unwrap()
    }

    async fn get_state(&self, state_id: BundleStateId) -> Option<StakingBundle> {
        let db = self.db.clone();

        let state_key = prefixed_key(STATE_PREFIX, &state_id);
        spawn_blocking(move || {
            db.get(state_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
        })
        .await
        .unwrap()
    }

    /// Invalidate bundle state snapshot corresponding to the given `state_id`.
    async fn invalidate(&self, state_id: BundleStateId) {
        let db = self.db.clone();
        let state_key = prefixed_key(STATE_PREFIX, &state_id);
        let link_key = prefixed_key(PREDICTION_LINK_PREFIX, &state_id);
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.delete(state_key).unwrap();
            tx.delete(link_key).unwrap();
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn put_confirmed(&self, Confirmed(bundle): Confirmed<AsBox<StakingBundle>>) {
        let db = self.db.clone();
        let state_id_bytes = bincode::serialize(&bundle.get_self_state_ref()).unwrap();
        let state_key = prefixed_key(STATE_PREFIX, &bundle.get_self_state_ref());
        let state_bytes = bincode::serialize(&bundle).unwrap();
        let index_key = prefixed_key(LAST_CONFIRMED_PREFIX, &bundle.get_self_ref());
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.put(state_key, state_bytes).unwrap();
            tx.put(index_key, state_id_bytes).unwrap();
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn put_unconfirmed(&self, Unconfirmed(bundle): Unconfirmed<AsBox<StakingBundle>>) {
        let db = self.db.clone();

        let state_id_bytes = bincode::serialize(&bundle.get_self_state_ref()).unwrap();
        let state_key = prefixed_key(STATE_PREFIX, &bundle.get_self_state_ref());
        let state_bytes = bincode::serialize(&bundle).unwrap();
        let index_key = prefixed_key(LAST_UNCONFIRMED_PREFIX, &bundle.get_self_ref());
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.put(state_key, state_bytes).unwrap();
            tx.put(index_key, state_id_bytes).unwrap();
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn put_predicted(
        &self,
        Traced {
            state: Predicted(bundle),
            prev_state_id,
        }: Traced<Predicted<AsBox<StakingBundle>>>,
    ) {
        let db = self.db.clone();

        let state_id_bytes = bincode::serialize(&bundle.get_self_state_ref()).unwrap();
        let state_key = prefixed_key(STATE_PREFIX, &bundle.get_self_state_ref());
        let state_bytes = bincode::serialize(&bundle).unwrap();
        let index_key = prefixed_key(LAST_PREDICTED_PREFIX, &bundle.get_self_ref());
        let link_key = prefixed_key(PREDICTION_LINK_PREFIX, &bundle.get_self_state_ref());
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.put(state_key, state_bytes).unwrap();
            tx.put(index_key, state_id_bytes).unwrap();
            if let Some(prev_sid) = prev_state_id {
                let prev_state_id_bytes = bincode::serialize(&prev_sid).unwrap();
                tx.put(link_key, prev_state_id_bytes).unwrap();
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap();
    }

    async fn get_last_confirmed(&self, id: BundleId) -> Option<Confirmed<AsBox<StakingBundle>>> {
        let db = self.db.clone();
        let index_key = prefixed_key(LAST_CONFIRMED_PREFIX, &id);
        spawn_blocking(move || {
            db.get(index_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize::<'_, BundleStateId>(&bytes).ok())
                .and_then(|sid| db.get(prefixed_key(STATE_PREFIX, &sid)).unwrap())
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
        })
        .await
        .unwrap()
    }

    async fn get_last_unconfirmed(&self, id: BundleId) -> Option<Unconfirmed<AsBox<StakingBundle>>> {
        let db = self.db.clone();
        let index_key = prefixed_key(LAST_UNCONFIRMED_PREFIX, &id);
        spawn_blocking(move || {
            db.get(index_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize::<'_, BundleStateId>(&bytes).ok())
                .and_then(|sid| db.get(prefixed_key(STATE_PREFIX, &sid)).unwrap())
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
        })
        .await
        .unwrap()
    }

    async fn get_last_predicted(&self, id: BundleId) -> Option<Predicted<AsBox<StakingBundle>>> {
        let db = self.db.clone();
        let index_key = prefixed_key(LAST_PREDICTED_PREFIX, &id);
        spawn_blocking(move || {
            db.get(index_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize::<'_, BundleStateId>(&bytes).ok())
                .and_then(|sid| db.get(prefixed_key(STATE_PREFIX, &sid)).unwrap())
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
        })
        .await
        .unwrap()
    }

    async fn get_prediction_predecessor(&self, id: BundleStateId) -> Option<BundleStateId> {
        let db = self.db.clone();

        let link_key = prefixed_key(PREDICTION_LINK_PREFIX, &id);
        spawn_blocking(move || {
            db.get(link_key)
                .unwrap()
                .and_then(|bytes| bincode::deserialize(&bytes).ok())
        })
        .await
        .unwrap()
    }
}

const STATE_PREFIX: &str = "state";
const PREDICTION_LINK_PREFIX: &str = "prediction:link";
const LAST_PREDICTED_PREFIX: &str = "predicted:last";
const LAST_CONFIRMED_PREFIX: &str = "confirmed:last";
const LAST_UNCONFIRMED_PREFIX: &str = "unconfirmed:last";
