use std::sync::Arc;

use futures::{Stream, StreamExt};
use tokio::sync::Mutex;

use spectrum_offchain::combinators::EitherOrBoth::{Both, Right};
use spectrum_offchain::data::unique_entity::{Confirmed, StateUpdate};
use spectrum_offchain::data::OnChainEntity;

use crate::data::pool::Pool;
use crate::data::AsBox;
use crate::program::ProgramRepo;

pub fn track_programs<'a, S, TRepo>(
    confirmed_pools: S,
    repo: Arc<Mutex<TRepo>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = Confirmed<StateUpdate<AsBox<Pool>>>> + 'a,
    TRepo: ProgramRepo + 'a,
{
    confirmed_pools.then(move |Confirmed(upd)| {
        let repo = Arc::clone(&repo);
        async move {
            if let StateUpdate::Transition(Right(pool) | Both(_, pool)) = upd {
                let repo = repo.lock().await;
                if !repo.exists(pool.get_self_ref()).await {
                    repo.put(pool.get_self_ref(), pool.1.conf).await;
                }
            }
        }
    })
}
