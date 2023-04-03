use std::sync::Arc;

use async_std::task::spawn_blocking;
use async_trait::async_trait;
use log::trace;
use nonempty::NonEmpty;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use serde::Serialize;

use ergo_chain_sync::rocksdb::RocksConfig;
use spectrum_offchain::binary::prefixed_key;
use spectrum_offchain::data::unique_entity::{Confirmed, Predicted};

use crate::data::funding::DistributionFunding;
use crate::data::{AsBox, FundingId};
use crate::ergo::{NanoErg, MAX_VALUE};

pub mod process;

#[async_trait(?Send)]
pub trait FundingRepo {
    /// Collect funding boxes that cover the specified `target`.
    async fn collect(&mut self, target: NanoErg) -> Result<NonEmpty<AsBox<DistributionFunding>>, ()>;
    async fn put_confirmed(&mut self, df: Confirmed<AsBox<DistributionFunding>>);
    async fn put_predicted(&mut self, df: Predicted<AsBox<DistributionFunding>>);
    /// False positive version of `exists()`.
    async fn may_exist(&self, fid: FundingId) -> bool;
    async fn remove(&mut self, fid: FundingId);
}

pub struct FundingRepoTracing<R> {
    inner: R,
}

impl<R> FundingRepoTracing<R> {
    pub fn wrap(repo: R) -> Self {
        Self { inner: repo }
    }
}

#[async_trait(?Send)]
impl<R> FundingRepo for FundingRepoTracing<R>
where
    R: FundingRepo,
{
    async fn collect(&mut self, target: NanoErg) -> Result<NonEmpty<AsBox<DistributionFunding>>, ()> {
        trace!(target: "funding", "collect(target: {:?})", target);
        let res = self.inner.collect(target).await;
        trace!(target: "funding", "collect(target: {:?}) -> {:?}", target, res);
        res
    }

    async fn put_confirmed(&mut self, df: Confirmed<AsBox<DistributionFunding>>) {
        let id = df.0 .1.id;
        trace!(target: "funding", "put_confirmed(df: {:?})", id);
        self.inner.put_confirmed(df).await;
        trace!(target: "funding", "put_confirmed(df: {:?}) -> ()", id);
    }

    async fn put_predicted(&mut self, df: Predicted<AsBox<DistributionFunding>>) {
        let id = df.0 .1.id;
        trace!(target: "funding", "put_predicted(df: {:?})", id);
        self.inner.put_predicted(df).await;
        trace!(target: "funding", "put_predicted(df: {:?}) -> ()", id);
    }

    async fn may_exist(&self, fid: FundingId) -> bool {
        self.inner.may_exist(fid).await
    }

    async fn remove(&mut self, fid: FundingId) {
        trace!(target: "funding", "remove(fid: {:?})", fid);
        self.inner.remove(fid).await;
        trace!(target: "funding", "remove(fid: {:?}) -> ()", fid);
    }
}

const FUNDING_KEY_PREFIX: &str = "funding";
const FUNDING_KEY_INDEX_PREFIX: &str = "funding_index";

fn funding_key<T: Serialize>(prefix: &str, seq_num: usize, value: u64, id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(prefix).unwrap();
    let seq_num_bytes = bincode::serialize(&seq_num).unwrap();
    let neg_value_bytes = bincode::serialize(&(MAX_VALUE - value)).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&seq_num_bytes);
    key_bytes.extend_from_slice(&neg_value_bytes);
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

mod db_models {
    use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
    use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
    use serde::{Deserialize, Serialize};

    use crate::data::funding;
    use crate::data::FundingId;
    use crate::ergo::NanoErg;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct DistributionFunding {
        pub id: FundingId,
        pub prop_bytes: Vec<u8>,
        pub erg_value: NanoErg,
    }

    impl From<funding::DistributionFunding> for DistributionFunding {
        fn from(df: funding::DistributionFunding) -> Self {
            Self {
                id: df.id,
                prop_bytes: df.prop.sigma_serialize_bytes().unwrap(),
                erg_value: df.erg_value,
            }
        }
    }

    impl From<DistributionFunding> for funding::DistributionFunding {
        fn from(df: DistributionFunding) -> Self {
            Self {
                id: df.id,
                prop: ErgoTree::sigma_parse_bytes(&df.prop_bytes).unwrap(),
                erg_value: df.erg_value,
            }
        }
    }
}

pub struct FundingRepoRocksDB {
    db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl FundingRepoRocksDB {
    pub fn new(conf: RocksConfig) -> Self {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(conf.db_path).unwrap()),
        }
    }
}

const CONFIRMED_PRIORITY: usize = 0;
const PREDICTED_PRIORITY: usize = 5;
const SELECTED_PRIORITY: usize = 10;

#[async_trait(?Send)]
impl FundingRepo for FundingRepoRocksDB {
    async fn collect(&mut self, target: NanoErg) -> Result<NonEmpty<AsBox<DistributionFunding>>, ()> {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            let prefix = bincode::serialize(FUNDING_KEY_PREFIX).unwrap();
            let mut readopts = ReadOptions::default();
            readopts.set_iterate_range(rocksdb::PrefixRange(prefix.clone()));
            let mut iter = db.iterator_opt(IteratorMode::From(&prefix, Direction::Forward), readopts);
            let mut funds = Vec::new();
            let mut acc = NanoErg::from(0);
            while let Some(Ok((key, bytes))) = iter.next() {
                if acc >= target {
                    break;
                }
                if let Ok(AsBox(bx, df)) =
                    bincode::deserialize::<'_, AsBox<db_models::DistributionFunding>>(&bytes)
                {
                    acc = acc + df.erg_value;
                    funds.push(AsBox(bx, DistributionFunding::from(df.clone())));

                    let new_key =
                        funding_key(FUNDING_KEY_PREFIX, SELECTED_PRIORITY, df.erg_value.into(), &df.id);
                    let index_key = prefixed_key(FUNDING_KEY_INDEX_PREFIX, &df.id);
                    let tx = db.transaction();
                    tx.delete(key).unwrap();
                    tx.put(new_key.clone(), bytes).unwrap();
                    tx.put(index_key, new_key).unwrap();
                    tx.commit().unwrap();
                }
            }
            if acc >= target {
                Ok(NonEmpty::from_vec(funds).unwrap())
            } else {
                trace!(target: "funding", "acc: {}, funds: {:?}", acc, funds);
                Err(())
            }
        })
        .await
    }

    async fn put_confirmed(&mut self, Confirmed(AsBox(bx, df)): Confirmed<AsBox<DistributionFunding>>) {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            let key = funding_key(
                FUNDING_KEY_PREFIX,
                CONFIRMED_PRIORITY,
                df.erg_value.into(),
                &df.id,
            );
            let index_key = prefixed_key(FUNDING_KEY_INDEX_PREFIX, &df.id);
            let df_as_box = AsBox(bx, db_models::DistributionFunding::from(df));
            let value = bincode::serialize(&df_as_box).unwrap();
            let tx = db.transaction();
            tx.put(key.clone(), value).unwrap();
            tx.put(index_key, key).unwrap();
            tx.commit().unwrap()
        })
        .await
    }

    async fn put_predicted(&mut self, Predicted(AsBox(bx, df)): Predicted<AsBox<DistributionFunding>>) {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            let key = funding_key(
                FUNDING_KEY_PREFIX,
                PREDICTED_PRIORITY,
                df.erg_value.into(),
                &df.id,
            );
            let index_key = prefixed_key(FUNDING_KEY_INDEX_PREFIX, &df.id);
            let df_as_box = AsBox(bx, db_models::DistributionFunding::from(df));
            let value = bincode::serialize(&df_as_box).unwrap();
            let tx = db.transaction();
            tx.put(key.clone(), value).unwrap();
            tx.put(index_key, key).unwrap();
            tx.commit().unwrap()
        })
        .await
    }

    async fn may_exist(&self, fid: FundingId) -> bool {
        let db = Arc::clone(&self.db);
        let index_key = prefixed_key(FUNDING_KEY_INDEX_PREFIX, &fid);
        spawn_blocking(move || db.key_may_exist(index_key)).await
    }

    async fn remove(&mut self, fid: FundingId) {
        let db = Arc::clone(&self.db);
        let index_key = prefixed_key(FUNDING_KEY_INDEX_PREFIX, &fid);
        spawn_blocking(move || {
            if let Some(key) = db.get(index_key.clone()).unwrap() {
                let tx = db.transaction();
                tx.delete(index_key).unwrap();
                tx.delete(key).unwrap();
                tx.commit().unwrap()
            }
        })
        .await
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use ergo_lib::chain::transaction::TxId;
    use ergo_lib::ergo_chain_types::Digest32;
    use ergo_lib::ergotree_ir::chain::ergo_box::box_value::BoxValue;
    use ergo_lib::ergotree_ir::chain::ergo_box::{BoxId, ErgoBox, NonMandatoryRegisters};
    use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
    use ergo_lib::ergotree_ir::mir::constant::Constant;
    use ergo_lib::ergotree_ir::mir::expr::Expr;
    use nonempty::NonEmpty;
    use rand::RngCore;

    use spectrum_offchain::data::unique_entity::{Confirmed, Predicted};

    use crate::data::funding::DistributionFunding;
    use crate::data::{AsBox, FundingId};
    use crate::ergo::NanoErg;
    use crate::funding::{FundingRepo, FundingRepoRocksDB};

    fn rocks_db_client() -> FundingRepoRocksDB {
        let rnd = rand::thread_rng().next_u32();
        FundingRepoRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
        }
    }

    fn trivial_prop() -> ErgoTree {
        ErgoTree::try_from(Expr::Const(Constant::from(true))).unwrap()
    }

    fn trivial_box() -> ErgoBox {
        ErgoBox::new(
            BoxValue::SAFE_USER_MIN,
            trivial_prop(),
            None,
            NonMandatoryRegisters::empty(),
            0,
            TxId::zero(),
            0,
        )
        .unwrap()
    }

    fn funding(ix: u8, amt: u64) -> AsBox<DistributionFunding> {
        AsBox(
            trivial_box(),
            DistributionFunding {
                id: FundingId::from(BoxId::from(Digest32::from([ix; 32]))),
                prop: trivial_prop(),
                erg_value: NanoErg::from(amt),
            },
        )
    }

    #[tokio::test]
    async fn collect_sucess() {
        let mut client = rocks_db_client();
        let f1 = funding(0, 1_000_000);
        let f2 = funding(1, 2_000_000);
        let f3 = funding(2, 4_000_000);
        for f in vec![f2, f1, f3] {
            client.put_confirmed(Confirmed(f)).await
        }
        let target = NanoErg::from(5_000_000);
        let res = client.collect(target).await.unwrap();
        let collected = res.into_iter().map(|AsBox(_, x)| x.erg_value).sum::<NanoErg>();
        assert!(collected >= target);
    }

    #[tokio::test]
    async fn collect_not_enough_funds() {
        let mut client = rocks_db_client();
        let f1 = funding(0, 1_000_000);
        let f2 = funding(1, 2_000_000);
        let f3 = funding(2, 4_000_000);
        for f in vec![f2, f1, f3] {
            client.put_confirmed(Confirmed(f)).await
        }
        let target = NanoErg::from(8_000_000);
        let res = client.collect(target).await;
        assert_eq!(res, Err(()));
    }

    #[tokio::test]
    async fn collect_confirmed_over_predicted() {
        let mut client = rocks_db_client();
        let f1 = funding(0, 1_000_000);
        let f2 = funding(1, 2_000_000);
        let f3 = funding(2, 4_000_000);
        let f4 = funding(3, 8_000_000);
        for f in vec![f1.clone(), f3.clone()] {
            client.put_confirmed(Confirmed(f)).await
        }
        for f in vec![f2, f4] {
            client.put_predicted(Predicted(f)).await
        }
        let target = NanoErg::from(5_000_000);
        let res = client.collect(target).await;
        assert_eq!(res, Ok(NonEmpty::from((f1, vec![f3]))));
    }
}
