use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::combinators::EitherOrBoth;
use crate::data::unique_entity::{Confirmed, StateUpdate};
use crate::data::OnChainEntity;

pub fn entity_tracking_stream<'a, TRepo, TEntity>(
    upstream: UnboundedReceiver<Confirmed<StateUpdate<TEntity>>>,
    entities: Arc<Mutex<TRepo>>,
) -> impl Stream<Item = ()> + 'a
where
    TEntity: OnChainEntity + 'a,
    TRepo: EntityRepo<TEntity> + 'a,
{
    upstream.then(move |Confirmed(upd)| {
        let entities = Arc::clone(&entities);
        async move {
            match upd {
                StateUpdate::Transition(EitherOrBoth::Right(new_state))
                | StateUpdate::Transition(EitherOrBoth::Both(_, new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Right(new_state))
                | StateUpdate::TransitionRollback(EitherOrBoth::Both(_, new_state)) => {
                    entities.lock().put_confirmed(Confirmed(new_state)).await
                }
                StateUpdate::Transition(EitherOrBoth::Left(st)) => entities.lock().eliminate(st).await,
                StateUpdate::TransitionRollback(EitherOrBoth::Left(st)) => {
                    entities.lock().invalidate(st.get_self_state_ref()).await
                }
            }
        }
    })
}
