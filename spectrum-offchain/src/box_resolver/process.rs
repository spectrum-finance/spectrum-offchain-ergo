use std::pin::Pin;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use futures::stream::select_all;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::data::unique_entity::{Confirmed, Unconfirmed, Upgrade, UpgradeRollback};
use crate::data::{Has, OnChainEntity};

pub fn box_tracker_stream<'a, TRepo, TEntity>(
    persistence: TRepo,
    conf_upgrades: UnboundedReceiver<Upgrade<Confirmed<TEntity>>>,
    unconf_upgrades: UnboundedReceiver<Upgrade<Unconfirmed<TEntity>>>,
    rollbacks: UnboundedReceiver<UpgradeRollback<TEntity>>,
) -> impl Stream<Item = ()> + 'a
where
    TRepo: EntityRepo<TEntity> + 'a,
    TEntity: OnChainEntity + Has<TEntity::TEntityId> + Has<TEntity::TStateId> + 'a,
{
    let persistence = Arc::new(Mutex::new(persistence));
    select_all(vec![
        track_conf_upgrades(persistence.clone(), conf_upgrades),
        track_unconf_upgrades(persistence.clone(), unconf_upgrades),
        handle_rollbacks(persistence, rollbacks),
    ])
}

#[allow(clippy::await_holding_lock)]
fn track_conf_upgrades<'a, TRepo, TEntity>(
    persistence: Arc<Mutex<TRepo>>,
    conf_upgrades: UnboundedReceiver<Upgrade<Confirmed<TEntity>>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TRepo: EntityRepo<TEntity> + 'a,
    TEntity: OnChainEntity + 'a,
{
    Box::pin(conf_upgrades.then(move |Upgrade(et)| {
        let pers = Arc::clone(&persistence);
        async move {
            let mut pers_guard = pers.lock();
            pers_guard.put_confirmed(et).await;
        }
    }))
}

#[allow(clippy::await_holding_lock)]
fn track_unconf_upgrades<'a, TRepo, TEntity>(
    persistence: Arc<Mutex<TRepo>>,
    unconf_upgrades: UnboundedReceiver<Upgrade<Unconfirmed<TEntity>>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TRepo: EntityRepo<TEntity> + 'a,
    TEntity: OnChainEntity + 'a,
{
    Box::pin(unconf_upgrades.then(move |Upgrade(et)| {
        let pers = Arc::clone(&persistence);
        async move {
            let mut pers_guard = pers.lock();
            pers_guard.put_unconfirmed(et).await;
        }
    }))
}

#[allow(clippy::await_holding_lock)]
fn handle_rollbacks<'a, TRepo, TEntity>(
    repo: Arc<Mutex<TRepo>>,
    rollbacks: UnboundedReceiver<UpgradeRollback<TEntity>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TRepo: EntityRepo<TEntity> + 'a,
    TEntity: OnChainEntity + Has<TEntity::TEntityId> + Has<TEntity::TStateId> + 'a,
{
    Box::pin(rollbacks.then(move |UpgradeRollback(et)| {
        let repo = Arc::clone(&repo);
        async move {
            repo.lock().invalidate(et.get::<TEntity::TStateId>()).await;
        }
    }))
}
