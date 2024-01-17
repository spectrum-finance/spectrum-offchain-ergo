use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use ergo_lib::chain::transaction::Transaction;
use futures::{Sink, SinkExt};
use tokio::sync::Mutex;

use spectrum_offchain::combinators::EitherOrBoth;
use spectrum_offchain::data::unique_entity::{Confirmed, StateUpdate};
use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::event_sink::handlers::types::TryFromBox;
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;

use crate::bundle::BundleRepo;
use crate::data::bundle::{IndexedBundle, IndexedStakingBundle, StakingBundle};
use crate::data::{AsBox, BundleId, BundleStateId};
use crate::program::ProgramRepo;

pub struct ConfirmedBundleUpdateHadler<TSink, TBundles, TProgs> {
    pub topic: TSink,
    pub bundles: Arc<Mutex<TBundles>>,
    pub programs: Arc<Mutex<TProgs>>,
}

impl<TSink, TBundles, TProgs> ConfirmedBundleUpdateHadler<TSink, TBundles, TProgs>
where
    TBundles: BundleRepo,
    TProgs: ProgramRepo,
{
    async fn extract_transitions(
        &self,
        tx: Transaction,
    ) -> Vec<EitherOrBoth<AsBox<IndexedStakingBundle>, AsBox<IndexedStakingBundle>>> {
        let mut consumed_bundles = HashMap::<BundleId, AsBox<IndexedStakingBundle>>::new();
        {
            let bundles = self.bundles.lock().await;
            for i in tx.clone().inputs {
                let state_id = BundleStateId::from(i.box_id);
                if bundles.may_exist(state_id).await {
                    if let Some(indexed_bundle) = bundles.get_state(state_id).await {
                        consumed_bundles.insert(indexed_bundle.get_self_ref(), indexed_bundle);
                    }
                }
            }
        }
        let mut created_bundles = HashMap::<BundleId, AsBox<IndexedStakingBundle>>::new();
        {
            let programs = self.programs.lock().await;
            for bx in &tx.outputs {
                if let Some(bundle) = StakingBundle::try_from_box(bx.clone()) {
                    let indexed_bundle = if let Some(prog) = programs.get(bundle.pool_id).await {
                        IndexedBundle::new(bundle, prog)
                    } else {
                        // handle initialization bundle
                        IndexedBundle::init(bundle)
                    };
                    created_bundles.insert(indexed_bundle.get_self_ref(), AsBox(bx.clone(), indexed_bundle));
                }
            }
        }
        let consumed_keys = consumed_bundles.keys().cloned().collect::<HashSet<_>>();
        let created_keys = created_bundles.keys().cloned().collect::<HashSet<_>>();
        consumed_keys
            .union(&created_keys)
            .flat_map(|k| {
                EitherOrBoth::try_from((consumed_bundles.remove(k), created_bundles.remove(k)))
                    .map(|x| vec![x])
                    .unwrap_or(Vec::new())
            })
            .collect()
    }
}

#[async_trait(? Send)]
impl<TSink, TBundles, TProgs> EventHandler<LedgerTxEvent>
    for ConfirmedBundleUpdateHadler<TSink, TBundles, TProgs>
where
    TSink: Sink<Confirmed<StateUpdate<AsBox<IndexedStakingBundle>>>> + Unpin,
    TBundles: BundleRepo,
    TProgs: ProgramRepo,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        let res = match ev {
            LedgerTxEvent::AppliedTx {
                tx,
                timestamp,
                height,
            } => {
                let transitions = self.extract_transitions(tx.clone()).await;
                let is_success = !transitions.is_empty();
                for tr in transitions {
                    let _ = self.topic.feed(Confirmed(StateUpdate::Transition(tr))).await;
                }
                if is_success {
                    Some(LedgerTxEvent::AppliedTx {
                        tx,
                        timestamp,
                        height,
                    })
                } else {
                    None
                }
            }
            LedgerTxEvent::UnappliedTx(tx) => {
                let transitions = self.extract_transitions(tx.clone()).await;
                let is_success = !transitions.is_empty();
                for tr in transitions {
                    let _ = self
                        .topic
                        .feed(Confirmed(StateUpdate::TransitionRollback(tr.swap())))
                        .await;
                }
                if is_success {
                    Some(LedgerTxEvent::UnappliedTx(tx))
                } else {
                    None
                }
            }
        };
        let _ = self.topic.flush().await;
        res
    }
}
