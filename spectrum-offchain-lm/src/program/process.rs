use std::sync::Arc;

use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use spectrum_offchain::combinators::EitherOrBoth::{Both, Right};
use spectrum_offchain::data::unique_entity::{Confirmed, StateUpdate};

use crate::data::pool::Pool;
use crate::program::ProgramRepo;

pub fn track_programs<'a, S, TRepo>(
    confirmed_pools: S,
    repo: Arc<Mutex<TRepo>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = Confirmed<StateUpdate<Pool>>> + 'a,
    TRepo: ProgramRepo + 'a,
{
    confirmed_pools.then(move |Confirmed(upd)| {
        let repo = Arc::clone(&repo);
        async move {
            if let StateUpdate::Transition(Right(pool) | Both(_, pool)) = upd {
                let repo = repo.lock();
                if !repo.exists(pool.pool_id).await {
                    repo.put(pool.pool_id, pool.conf).await;
                }
            }
        }
    })
}
