use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use spectrum_offchain::data::unique_entity::{Confirmed, StateUpdate};
use spectrum_offchain::data::OnChainEntity;

use crate::bundle::BundleRepo;
use crate::data::bundle::IndexedStakingBundle;
use crate::data::AsBox;

pub fn bundle_update_stream<'a, TBundles>(
    upstream: UnboundedReceiver<Confirmed<StateUpdate<AsBox<IndexedStakingBundle>>>>,
    bundles: Arc<Mutex<TBundles>>,
) -> impl Stream<Item = ()> + 'a
where
    TBundles: BundleRepo + 'a,
{
    upstream.then(move |Confirmed(upd)| {
        let bundles = Arc::clone(&bundles);
        async move {
            match upd {
                StateUpdate::Transition {
                    new_state: Some(new_state),
                    ..
                }
                | StateUpdate::TransitionRollback {
                    revived_state: Some(new_state),
                    ..
                } => bundles.lock().put_confirmed(Confirmed(new_state)).await,
                StateUpdate::Transition {
                    old_state: Some(AsBox(_, st)),
                    new_state: None,
                } => bundles.lock().eliminate(st).await,
                StateUpdate::TransitionRollback {
                    rolled_back_state: Some(AsBox(_, st)),
                    ..
                } => bundles.lock().invalidate(st.get_self_state_ref()).await,
                _ => {}
            }
        }
    })
}
