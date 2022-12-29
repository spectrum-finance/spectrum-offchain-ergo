use std::sync::Arc;

use futures::{Stream, StreamExt};
use tokio::sync::Mutex;

use spectrum_offchain::combinators::EitherOrBoth;
use spectrum_offchain::data::unique_entity::{Confirmed, StateUpdate};
use spectrum_offchain::data::OnChainEntity;

use crate::bundle::BundleRepo;
use crate::data::bundle::IndexedStakingBundle;
use crate::data::AsBox;

pub fn bundle_update_stream<'a, S, TBundles>(
    upstream: S,
    bundles: Arc<Mutex<TBundles>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = Confirmed<StateUpdate<AsBox<IndexedStakingBundle>>>> + 'a,
    TBundles: BundleRepo + 'a,
{
    upstream.then(move |Confirmed(upd)| {
        let bundles = Arc::clone(&bundles);
        async move {
            let bundles = bundles.lock().await;
            match upd {
                StateUpdate::Transition(EitherOrBoth::Right(new_state))
                | StateUpdate::Transition(EitherOrBoth::Both(_, new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Right(new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Both(_, new_state)) => {
                    bundles.put_confirmed(Confirmed(new_state)).await
                }
                StateUpdate::Transition(EitherOrBoth::Left(AsBox(_, st))) => bundles.eliminate(st).await,
                StateUpdate::TransitionRollback(EitherOrBoth::Left(AsBox(_, st))) => {
                    bundles.invalidate(st.get_self_state_ref()).await
                }
            }
        }
    })
}
