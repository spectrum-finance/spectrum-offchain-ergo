use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use ergo_lib::chain::transaction::Transaction;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;

use spectrum_offchain::combinators::EitherOrBoth;
use spectrum_offchain::data::unique_entity::{Confirmed, StateUpdate};
use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::event_sink::handlers::types::TryFromBox;
use spectrum_offchain::event_sink::types::EventHandler;
use spectrum_offchain::event_source::data::LedgerTxEvent;

use crate::bundle::BundleRepo;
use crate::data::bundle::{IndexedBundle, IndexedStakingBundle, StakingBundle};
use crate::data::{AsBox, BundleId, BundleStateId};
use crate::pool::ProgramRepo;

pub struct ConfirmedBundleUpdateHadler<TBundles, TProgs> {
    topic: UnboundedSender<Confirmed<StateUpdate<AsBox<IndexedStakingBundle>>>>,
    bundles: Arc<Mutex<TBundles>>,
    programs: Arc<Mutex<TProgs>>,
}

impl<TBundles, TProgs> ConfirmedBundleUpdateHadler<TBundles, TProgs>
where
    TBundles: BundleRepo,
    TProgs: ProgramRepo,
{
    async fn extract_transitions(
        &self,
        tx: Transaction,
    ) -> Vec<EitherOrBoth<AsBox<IndexedStakingBundle>, AsBox<IndexedStakingBundle>>> {
        let mut consumed_bundles = HashMap::<BundleId, AsBox<IndexedStakingBundle>>::new();
        for i in tx.clone().inputs {
            let state_id = BundleStateId::from(i.box_id);
            let bundles = self.bundles.lock();
            if bundles.may_exist(state_id).await {
                if let Some(indexed_bundle) = bundles.get_state(state_id).await {
                    consumed_bundles.insert(indexed_bundle.get_self_ref(), indexed_bundle);
                }
            }
        }
        let mut created_bundles = HashMap::<BundleId, AsBox<IndexedStakingBundle>>::new();
        for bx in &tx.outputs {
            if let Some(bundle) = StakingBundle::try_from_box(bx.clone()) {
                if let Some(prog) = self.programs.lock().get(bundle.pool_id).await {
                    let indexed_bundle = IndexedBundle::new(bundle, prog);
                    created_bundles.insert(indexed_bundle.get_self_ref(), AsBox(bx.clone(), indexed_bundle));
                }
            }
        }
        let consumed_keys = consumed_bundles.keys().cloned().collect::<HashSet<_>>();
        let created_keys = created_bundles.keys().cloned().collect::<HashSet<_>>();
        consumed_keys
            .union(&created_keys)
            .map(|k| {
                EitherOrBoth::try_from((consumed_bundles.remove(k), created_bundles.remove(k)))
                    .map(|x| vec![x])
                    .unwrap_or(Vec::new())
            })
            .flatten()
            .collect()
    }
}

#[async_trait(?Send)]
impl<TBundles, TProgs> EventHandler<LedgerTxEvent> for ConfirmedBundleUpdateHadler<TBundles, TProgs>
where
    TBundles: BundleRepo,
    TProgs: ProgramRepo,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let transitions = self.extract_transitions(tx.clone()).await;
                let is_success = transitions.len() > 0;
                for tr in transitions {
                    self.topic.send(Confirmed(StateUpdate::Transition(tr))).unwrap();
                }
                if is_success {
                    Some(LedgerTxEvent::AppliedTx { tx, timestamp })
                } else {
                    None
                }
            }
            LedgerTxEvent::UnappliedTx(tx) => {
                let transitions = self.extract_transitions(tx.clone()).await;
                let is_success = transitions.len() > 0;
                for tr in transitions {
                    self.topic
                        .send(Confirmed(StateUpdate::TransitionRollback(tr.swap())))
                        .unwrap();
                }
                if is_success {
                    Some(LedgerTxEvent::UnappliedTx(tx))
                } else {
                    None
                }
            }
        }
    }
}
