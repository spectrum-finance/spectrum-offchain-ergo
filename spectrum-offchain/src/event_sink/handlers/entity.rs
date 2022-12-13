use async_trait::async_trait;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use tokio::sync::mpsc::UnboundedSender;

use ergo_mempool_sync::MempoolUpdate;

use crate::box_resolver::persistence::EntityRepo;
use crate::data::unique_entity::{Confirmed, Unconfirmed, Upgrade, UpgradeRollback};
use crate::data::OnChainEntity;
use crate::event_sink::handlers::types::TryFromBox;
use crate::event_sink::types::EventHandler;
use crate::event_source::data::LedgerTxEvent;

pub struct ConfirmedUpgradeHandler<TEntity> {
    topic: UnboundedSender<Upgrade<Confirmed<TEntity>>>,
}

#[async_trait(?Send)]
impl<TEntity> EventHandler<LedgerTxEvent> for ConfirmedUpgradeHandler<TEntity>
where
    TEntity: TryFromBox,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for bx in &tx.outputs {
                    if let Some(order) = TEntity::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self.topic.send(Upgrade(Confirmed(order)));
                    }
                }
                if is_success {
                    return None;
                }
                Some(LedgerTxEvent::AppliedTx { tx, timestamp })
            }
            ev => Some(ev),
        }
    }
}

pub struct UnconfirmedUpgradeHandler<TEntity> {
    topic: UnboundedSender<Upgrade<Unconfirmed<TEntity>>>,
}

#[async_trait(?Send)]
impl<TEntity> EventHandler<MempoolUpdate> for UnconfirmedUpgradeHandler<TEntity>
where
    TEntity: TryFromBox,
{
    async fn try_handle(&mut self, ev: MempoolUpdate) -> Option<MempoolUpdate> {
        match ev {
            MempoolUpdate::TxAccepted(tx) => {
                let mut is_success = false;
                for bx in &tx.outputs {
                    if let Some(order) = TEntity::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self.topic.send(Upgrade(Unconfirmed(order)));
                    }
                }
                if is_success {
                    return None;
                }
                Some(MempoolUpdate::TxAccepted(tx))
            }
            ev => Some(ev),
        }
    }
}

pub struct ConfirmedRollbackHandler<TEntity, TRepo> {
    topic: UnboundedSender<UpgradeRollback<TEntity>>,
    repo: TRepo,
}

#[async_trait(?Send)]
impl<TEntity, TRepo> EventHandler<LedgerTxEvent> for ConfirmedRollbackHandler<TEntity, TRepo>
where
    TEntity: OnChainEntity + TryFromBox,
    TEntity::TStateId: From<BoxId>,
    TRepo: EntityRepo<TEntity>,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        match ev {
            LedgerTxEvent::AppliedTx { tx, timestamp } => {
                let mut is_success = false;
                for i in tx.clone().inputs {
                    let state_id = TEntity::TStateId::from(i.box_id);
                    if let Some(entity_snapshot) = self.repo.get_state(state_id).await {
                        is_success = true;
                        let _ = self.topic.send(UpgradeRollback(entity_snapshot));
                    }
                }
                if is_success {
                    return None;
                }
                Some(LedgerTxEvent::AppliedTx { tx, timestamp })
            }
            LedgerTxEvent::UnappliedTx(tx) => {
                let mut is_success = false;
                for bx in &tx.outputs {
                    if let Some(entity) = TEntity::try_from_box(bx.clone()) {
                        is_success = true;
                        let _ = self.topic.send(UpgradeRollback(entity));
                    }
                }
                if is_success {
                    return None;
                }
                Some(LedgerTxEvent::UnappliedTx(tx))
            }
        }
    }
}

pub struct UnconfirmedRollbackHandler<TEntity, TRepo> {
    topic: UnboundedSender<UpgradeRollback<TEntity>>,
    repo: TRepo,
}

#[async_trait(?Send)]
impl<TEntity, TRepo> EventHandler<MempoolUpdate> for UnconfirmedRollbackHandler<TEntity, TRepo>
where
    TEntity: OnChainEntity + TryFromBox,
    TEntity::TStateId: From<BoxId>,
    TRepo: EntityRepo<TEntity>,
{
    async fn try_handle(&mut self, ev: MempoolUpdate) -> Option<MempoolUpdate> {
        match ev {
            MempoolUpdate::TxAccepted(tx) => {
                // entity is consumed by another tx in mempool
                let mut is_success = false;
                for i in tx.clone().inputs {
                    let state_id = TEntity::TStateId::from(i.box_id);
                    if let Some(entity) = self.repo.get_state(state_id).await {
                        is_success = true;
                        let _ = self.topic.send(UpgradeRollback(entity));
                    }
                }
                if is_success {
                    return None;
                }
                Some(MempoolUpdate::TxAccepted(tx))
            }
            MempoolUpdate::TxWithdrawn(tx) => {
                // entity tx is dropped from mempool
                let mut is_success = false;
                for bx in tx.clone().outputs {
                    if let Some(entity) = TEntity::try_from_box(bx) {
                        is_success = true;
                        let _ = self.topic.send(UpgradeRollback(entity));
                    }
                }
                if is_success {
                    return None;
                }
                Some(MempoolUpdate::TxWithdrawn(tx))
            }
        }
    }
}
