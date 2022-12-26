use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use futures::{Sink, SinkExt};
use parking_lot::Mutex;

use ergo_mempool_sync::MempoolUpdate;

use crate::box_resolver::persistence::EntityRepo;
use crate::combinators::EitherOrBoth;
use crate::data::unique_entity::{Confirmed, StateUpdate, Unconfirmed};
use crate::data::OnChainEntity;
use crate::event_sink::handlers::types::TryFromBox;
use crate::event_sink::types::EventHandler;
use crate::event_source::data::LedgerTxEvent;

pub struct ConfirmedUpdateHandler<TSink, TEntity, TRepo> {
    pub topic: TSink,
    pub entities: Arc<Mutex<TRepo>>,
    pub pd: PhantomData<TEntity>,
}

impl<TSink, TEntity, TRepo> ConfirmedUpdateHandler<TSink, TEntity, TRepo> {
    pub fn new(topic: TSink, entities: Arc<Mutex<TRepo>>) -> Self {
        Self {
            topic,
            entities,
            pd: Default::default(),
        }
    }
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
impl<TSink, TEntity, TRepo> EventHandler<LedgerTxEvent> for ConfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TSink: Sink<Confirmed<StateUpdate<TEntity>>> + Unpin,
    TEntity: OnChainEntity + TryFromBox + Clone + Debug,
    TEntity::TEntityId: Clone,
    TEntity::TStateId: From<BoxId> + Copy,
    TRepo: EntityRepo<TEntity>,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        let res = match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let is_success = transitions.len() > 0;
                for tr in transitions {
                    let _ = self.topic.feed(Confirmed(StateUpdate::Transition(tr))).await;
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

pub struct UnconfirmedUpgradeHandler<TSink, TEntity, TRepo> {
    pub topic: TSink,
    pub entities: Arc<Mutex<TRepo>>,
    pub pd: PhantomData<TEntity>,
}

#[async_trait(?Send)]
impl<TSink, TEntity, TRepo> EventHandler<MempoolUpdate> for UnconfirmedUpgradeHandler<TSink, TEntity, TRepo>
where
    TSink: Sink<Unconfirmed<StateUpdate<TEntity>>> + Unpin,
    TEntity: OnChainEntity + TryFromBox + Clone + Debug,
    TEntity::TEntityId: Clone,
    TEntity::TStateId: From<BoxId> + Copy,
    TRepo: EntityRepo<TEntity>,
{
    async fn try_handle(&mut self, ev: MempoolUpdate) -> Option<MempoolUpdate> {
        let res = match ev {
            MempoolUpdate::TxAccepted(tx) => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let is_success = transitions.len() > 0;
                for tr in transitions {
                    let _ = self.topic.feed(Unconfirmed(StateUpdate::Transition(tr))).await;
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
                    let _ = self
                        .topic
                        .feed(Unconfirmed(StateUpdate::TransitionRollback(tr.swap())))
                        .await;
                }
                if is_success {
                    Some(MempoolUpdate::TxWithdrawn(tx))
                } else {
                    None
                }
            }
        };
        let _ = self.topic.flush().await;
        res
    }
}
