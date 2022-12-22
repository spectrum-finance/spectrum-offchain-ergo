use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;

use ergo_mempool_sync::MempoolUpdate;

use crate::box_resolver::persistence::EntityRepo;
use crate::combinators::EitherOrBoth;
use crate::data::unique_entity::{Confirmed, StateUpdate, Unconfirmed};
use crate::data::OnChainEntity;
use crate::event_sink::handlers::types::TryFromBox;
use crate::event_sink::types::EventHandler;
use crate::event_source::data::LedgerTxEvent;

pub struct ConfirmedUpdateHandler<TEntity, TRepo> {
    topic: UnboundedSender<Confirmed<StateUpdate<TEntity>>>,
    entities: Arc<Mutex<TRepo>>,
}

async fn extract_transitions<TEntity, TRepo>(
    entities: Arc<Mutex<TRepo>>,
    tx: Transaction,
) -> Vec<EitherOrBoth<TEntity, TEntity>>
where
    TEntity: OnChainEntity + TryFromBox + Clone,
    TEntity::TEntityId: Clone,
    TEntity::TStateId: From<BoxId> + Copy,
    TRepo: EntityRepo<TEntity>,
{
    let mut consumed_entities = HashMap::<TEntity::TEntityId, TEntity>::new();
    for i in tx.clone().inputs {
        let state_id = TEntity::TStateId::from(i.box_id);
        let entities = entities.lock();
        if entities.may_exist(state_id).await {
            if let Some(entity) = entities.get_state(state_id).await {
                consumed_entities.insert(entity.get_self_ref(), entity);
            }
        }
    }
    let mut created_entities = HashMap::<TEntity::TEntityId, TEntity>::new();
    for bx in &tx.outputs {
        if let Some(entity) = TEntity::try_from_box(bx.clone()) {
            created_entities.insert(entity.get_self_ref(), entity);
        }
    }
    let consumed_keys = consumed_entities.keys().cloned().collect::<HashSet<_>>();
    let created_keys = created_entities.keys().cloned().collect::<HashSet<_>>();
    consumed_keys
        .union(&created_keys)
        .map(|k| {
            EitherOrBoth::try_from((consumed_entities.remove(k), created_entities.remove(k)))
                .map(|x| vec![x])
                .unwrap_or(Vec::new())
        })
        .flatten()
        .collect()
}

#[async_trait(?Send)]
impl<TEntity, TRepo> EventHandler<LedgerTxEvent> for ConfirmedUpdateHandler<TEntity, TRepo>
where
    TEntity: OnChainEntity + TryFromBox + Clone + Debug,
    TEntity::TEntityId: Clone,
    TEntity::TStateId: From<BoxId> + Copy,
    TRepo: EntityRepo<TEntity>,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
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
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
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

pub struct UnconfirmedUpgradeHandler<TEntity, TRepo> {
    topic: UnboundedSender<Unconfirmed<StateUpdate<TEntity>>>,
    entities: Arc<Mutex<TRepo>>,
}

#[async_trait(?Send)]
impl<TEntity, TRepo> EventHandler<MempoolUpdate> for UnconfirmedUpgradeHandler<TEntity, TRepo>
where
    TEntity: OnChainEntity + TryFromBox + Clone + Debug,
    TEntity::TEntityId: Clone,
    TEntity::TStateId: From<BoxId> + Copy,
    TRepo: EntityRepo<TEntity>,
{
    async fn try_handle(&mut self, ev: MempoolUpdate) -> Option<MempoolUpdate> {
        match ev {
            MempoolUpdate::TxAccepted(tx) => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let is_success = transitions.len() > 0;
                for tr in transitions {
                    self.topic.send(Unconfirmed(StateUpdate::Transition(tr))).unwrap();
                }
                if is_success {
                    Some(MempoolUpdate::TxAccepted(tx))
                } else {
                    None
                }
            }
            MempoolUpdate::TxWithdrawn(tx) => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let is_success = transitions.len() > 0;
                for tr in transitions {
                    self.topic
                        .send(Unconfirmed(StateUpdate::TransitionRollback(tr.swap())))
                        .unwrap();
                }
                if is_success {
                    Some(MempoolUpdate::TxWithdrawn(tx))
                } else {
                    None
                }
            }
        }
    }
}
