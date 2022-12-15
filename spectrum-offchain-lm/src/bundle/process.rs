use std::pin::Pin;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use futures::stream::select_all;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use spectrum_offchain::data::unique_entity::{Confirmed, Upgrade, UpgradeRollback};

use crate::bundle::data::StakingBundle;
use crate::bundle::BundleRepo;
use crate::data::AsBox;

pub fn bundle_update_stream<'a, TBundles>(
    upgrades: UnboundedReceiver<Upgrade<Confirmed<AsBox<StakingBundle>>>>,
    rollbacks: UnboundedReceiver<UpgradeRollback<StakingBundle>>,
    bundles: Arc<Mutex<TBundles>>,
) -> impl Stream<Item = ()> + 'a
where
    TBundles: BundleRepo + 'a,
{
    select_all(vec![
        track_confirmed_bundle_upgrades(upgrades, Arc::clone(&bundles)),
        track_confirmed_bundle_rollbacks(rollbacks, bundles),
    ])
}

fn track_confirmed_bundle_upgrades<'a, TBundles>(
    upstream: UnboundedReceiver<Upgrade<Confirmed<AsBox<StakingBundle>>>>,
    bundles: Arc<Mutex<TBundles>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TBundles: BundleRepo + 'a,
{
    Box::pin(upstream.then(move |Upgrade(confirmed_bundle)| {
        let bundles = Arc::clone(&bundles);
        async move {
            bundles.lock().put_confirmed(confirmed_bundle).await;
        }
    }))
}

fn track_confirmed_bundle_rollbacks<'a, TBundles>(
    upstream: UnboundedReceiver<UpgradeRollback<StakingBundle>>,
    bundles: Arc<Mutex<TBundles>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TBundles: BundleRepo + 'a,
{
    Box::pin(upstream.then(move |UpgradeRollback(bundle)| {
        let bundles = Arc::clone(&bundles);
        async move {
            bundles.lock().invalidate(bundle.state_id).await;
        }
    }))
}
