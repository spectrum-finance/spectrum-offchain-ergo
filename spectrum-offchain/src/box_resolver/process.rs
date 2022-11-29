use std::pin::Pin;

use futures::channel::mpsc::UnboundedReceiver;
use futures::stream::select_all;
use futures::{Stream, StreamExt};

use crate::box_resolver::persistence::EntityRepo;
use crate::data::event::{Upgrade, UpgradeRollback};
use crate::data::state::{Confirmed, Unconfirmed};
use crate::data::Has;

pub fn box_tracker_stream<'a, TRepo, TEntity, TEntityId, TStateId>(
    persistence: TRepo,
    conf_upgrades: UnboundedReceiver<Upgrade<Confirmed<TEntity>>>,
    unconf_upgrades: UnboundedReceiver<Upgrade<Unconfirmed<TEntity>>>,
    rollbacks: UnboundedReceiver<UpgradeRollback<TEntity>>,
) -> impl Stream<Item = ()> + 'a
where
    TRepo: EntityRepo<TEntity, TEntityId, TStateId> + Clone + 'a,
    TEntity: Has<TEntityId> + Has<TStateId> + 'a,
{
    select_all(vec![
        track_conf_upgrades(persistence.clone(), conf_upgrades),
        track_unconf_upgrades(persistence.clone(), unconf_upgrades),
        handle_rollbacks(persistence.clone(), rollbacks),
    ])
}

fn track_conf_upgrades<'a, TRepo, TEntity, TEntityId, TStateId>(
    persistence: TRepo,
    conf_upgrades: UnboundedReceiver<Upgrade<Confirmed<TEntity>>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TRepo: EntityRepo<TEntity, TEntityId, TStateId> + Clone + 'a,
    TEntity: 'a,
{
    Box::pin(conf_upgrades.then(move |Upgrade(et)| {
        let mut p = persistence.clone();
        async move {
            p.put_confirmed(et).await;
        }
    }))
}

fn track_unconf_upgrades<'a, TRepo, TEntity, TEntityId, TStateId>(
    persistence: TRepo,
    unconf_upgrades: UnboundedReceiver<Upgrade<Unconfirmed<TEntity>>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TRepo: EntityRepo<TEntity, TEntityId, TStateId> + Clone + 'a,
    TEntity: 'a,
{
    Box::pin(unconf_upgrades.then(move |Upgrade(et)| {
        let mut p = persistence.clone();
        async move {
            p.put_unconfirmed(et).await;
        }
    }))
}

fn handle_rollbacks<'a, TRepo, TEntity, TEntityId, TStateId>(
    persistence: TRepo,
    rollbacks: UnboundedReceiver<UpgradeRollback<TEntity>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TRepo: EntityRepo<TEntity, TEntityId, TStateId> + Clone + 'a,
    TEntity: Has<TEntityId> + Has<TStateId> + 'a,
{
    Box::pin(rollbacks.then(move |UpgradeRollback(et)| {
        let mut p = persistence.clone();
        async move {
            p.invalidate(et.get::<TEntityId>(), et.get::<TStateId>()).await;
        }
    }))
}
