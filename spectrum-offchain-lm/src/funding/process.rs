use std::sync::Arc;

use futures::{Stream, StreamExt};
use tokio::sync::Mutex;

use spectrum_offchain::data::unique_entity::Confirmed;

use crate::data::funding::FundingUpdate;
use crate::funding::FundingRepo;

pub fn funding_update_stream<'a, S, TRepo>(
    upstream: S,
    repo: Arc<Mutex<TRepo>>,
) -> impl Stream<Item = ()> + 'a
where
    S: Stream<Item = Confirmed<FundingUpdate>> + 'a,
    TRepo: FundingRepo + 'a,
{
    upstream.then(move |Confirmed(upd)| {
        let repo = Arc::clone(&repo);
        async move {
            let mut repo = repo.lock().await;
            match upd {
                FundingUpdate::FundingCreated(funding) => repo.put_confirmed(Confirmed(funding)).await,
                FundingUpdate::FundingEliminated(fid) => repo.remove(fid).await,
            }
        }
    })
}
