use std::sync::Arc;

use async_trait::async_trait;
use itertools::FoldWhile::{Continue, Done};
use itertools::{FoldWhile, Itertools};
use rocksdb::{Direction, IteratorMode};
use tokio::task::spawn_blocking;

use spectrum_offchain::{
    binary::prefixed_key,
    data::{
        unique_entity::{Confirmed, Predicted, Traced, Unconfirmed},
        OnChainEntity,
    },
};

use crate::data::bundle::IndexedBundle;
use crate::data::{AsBox, BundleId, BundleStateId, PoolId};

use super::{BundleRepo, StakingBundle};

pub struct BundleRepoRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
    pub epoch_len: u32,
}

fn epoch_index_prefix(pool_id: PoolId, epoch_ix: u32) -> Vec<u8> {
    let mut prefix_bytes = bincode::serialize(POOL_EPOCH_PREFIX).unwrap();
    let pool_id_bytes = bincode::serialize(&pool_id).unwrap();
    let epoch_ix_bytes = bincode::serialize(&epoch_ix).unwrap();
    prefix_bytes.extend_from_slice(&pool_id_bytes);
    prefix_bytes.extend_from_slice(&epoch_ix_bytes);
    prefix_bytes
}

fn epoch_index_key(pool_id: PoolId, epoch_ix: u32, bundle_id: BundleId) -> Vec<u8> {
    let mut prefix_bytes = epoch_index_prefix(pool_id, epoch_ix);
    let bundle_id_bytes = bincode::serialize(&bundle_id).unwrap();
    prefix_bytes.extend_from_slice(&bundle_id_bytes);
    prefix_bytes
}

#[async_trait]
impl BundleRepo for BundleRepoRocksDB {
    async fn select(&self, pool_id: PoolId, epoch_ix: u32) -> Vec<BundleId> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let prefix = epoch_index_prefix(pool_id, epoch_ix);
            let mut acc = Vec::new();
            while let Some(Ok((key_bytes, _))) = db
                .iterator(IteratorMode::From(&*prefix, Direction::Forward))
                .next()
            {
                if key_bytes.len() == POOL_EPOCH_KEY_LEN {
                    let bundle_id =
                        bincode::deserialize::<'_, BundleId>(&key_bytes[50..POOL_EPOCH_KEY_LEN]).ok();
                    let init_epoch_ix = bincode::deserialize::<'_, u32>(&key_bytes[46..50]).ok();
                    if let (Some(bundle_id), Some(init_epoch_ix)) = (bundle_id, init_epoch_ix) {
                        if init_epoch_ix <= epoch_ix {
                            acc.push(bundle_id);
                            continue;
                        }
                    }
                }
                break;
            }
            acc
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

    async fn put_confirmed(&self, Confirmed(bundle_state): Confirmed<AsBox<IndexedBundle<StakingBundle>>>) {
        let db = self.db.clone();
        let state_id_bytes = bincode::serialize(&bundle_state.get_self_state_ref()).unwrap();
        let state_key = prefixed_key(STATE_PREFIX, &bundle_state.get_self_state_ref());
        let state_bytes = bincode::serialize(&bundle_state).unwrap();
        let index_key = prefixed_key(LAST_CONFIRMED_PREFIX, &bundle_state.get_self_ref());
        let epoch_index_key = epoch_index_key(
            bundle_state.1.bundle.pool_id,
            bundle_state.1.init_epoch_ix,
            bundle_state.get_self_ref(),
        );
        let dummy_bytes = vec![0u8];
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.put(state_key, state_bytes).unwrap();
            tx.put(index_key, state_id_bytes).unwrap();
            tx.put(epoch_index_key, dummy_bytes).unwrap();
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn put_predicted(
        &self,
        Traced {
            state: Predicted(bundle_state),
            prev_state_id,
        }: Traced<Predicted<AsBox<IndexedBundle<StakingBundle>>>>,
    ) {
        let db = self.db.clone();

        let state_id_bytes = bincode::serialize(&bundle_state.get_self_state_ref()).unwrap();
        let state_key = prefixed_key(STATE_PREFIX, &bundle_state.get_self_state_ref());
        let state_bytes = bincode::serialize(&bundle_state).unwrap();
        let index_key = prefixed_key(LAST_PREDICTED_PREFIX, &bundle_state.get_self_ref());
        let link_key = prefixed_key(PREDICTION_LINK_PREFIX, &bundle_state.get_self_state_ref());
        let epoch_index_key = epoch_index_key(
            bundle_state.1.bundle.pool_id,
            bundle_state.1.init_epoch_ix,
            bundle_state.get_self_ref(),
        );
        let dummy_bytes = vec![0u8];
        spawn_blocking(move || {
            let tx = db.transaction();
            tx.put(state_key, state_bytes).unwrap();
            tx.put(index_key, state_id_bytes).unwrap();
            tx.put(epoch_index_key, dummy_bytes).unwrap();
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
const PREDICTION_LINK_PREFIX: &str = "p:link";
const LAST_PREDICTED_PREFIX: &str = "p:last";
const LAST_CONFIRMED_PREFIX: &str = "c:last";
// Key structure: {prefix}{pool_id}{init_epoch_ix}{bundle_id}
const POOL_EPOCH_PREFIX: &str = "pl:epix";
const POOL_EPOCH_KEY_LEN: usize = 82;
