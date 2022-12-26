use std::pin::Pin;
use std::sync::Arc;

use futures::stream::select_all;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use spectrum_offchain::data::unique_entity::Confirmed;

use crate::data::funding::{DistributionFunding, EliminatedFunding};
use crate::data::AsBox;
use crate::funding::FundingRepo;

pub fn funding_update_stream<'a, S1, S2, TRepo>(
    new: S1,
    eliminated: S2,
    repo: Arc<Mutex<TRepo>>,
) -> impl Stream<Item = ()> + 'a
where
    S1: Stream<Item = Confirmed<AsBox<DistributionFunding>>> + 'a,
    S2: Stream<Item = EliminatedFunding> + 'a,
    TRepo: FundingRepo + 'a,
{
    select_all(vec![
        track_new_funding_boxes(new, Arc::clone(&repo)),
        track_eliminated_funding_boxes(eliminated, repo),
    ])
}

fn track_new_funding_boxes<'a, S, TRepo>(
    upstream: S,
    repo: Arc<Mutex<TRepo>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    S: Stream<Item = Confirmed<AsBox<DistributionFunding>>> + 'a,
    TRepo: FundingRepo + 'a,
{
    Box::pin(upstream.then(move |funding| {
        let repo = Arc::clone(&repo);
        async move { repo.lock().put_confirmed(funding).await }
    }))
}

fn track_eliminated_funding_boxes<'a, S, TRepo>(
    upstream: S,
    repo: Arc<Mutex<TRepo>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    S: Stream<Item = EliminatedFunding> + 'a,
    TRepo: FundingRepo + 'a,
{
    Box::pin(upstream.then(move |EliminatedFunding(fid)| {
        let repo = Arc::clone(&repo);
        async move { repo.lock().remove(fid).await }
    }))
}
