use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use spectrum_offchain::combinators::EitherOrBoth;
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
                StateUpdate::Transition(EitherOrBoth::Right(new_state))
                | StateUpdate::Transition(EitherOrBoth::Both(_, new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Right(new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Both(_, new_state)) => {
                    bundles.lock().put_confirmed(Confirmed(new_state)).await
                }
                StateUpdate::Transition(EitherOrBoth::Left(AsBox(_, st))) => {
                    bundles.lock().eliminate(st).await
                }
                StateUpdate::TransitionRollback(EitherOrBoth::Left(AsBox(_, st))) => {
                    bundles.lock().invalidate(st.get_self_state_ref()).await
                }
            }
        }
    })
}
