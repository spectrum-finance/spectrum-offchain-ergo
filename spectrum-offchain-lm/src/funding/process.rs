use std::pin::Pin;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use futures::stream::select_all;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use spectrum_offchain::data::unique_entity::Confirmed;

use crate::data::funding::{DistributionFunding, EliminatedFunding};
use crate::data::AsBox;
use crate::funding::FundingRepo;

pub fn funding_update_stream<'a, TRepo>(
    new: UnboundedReceiver<Confirmed<AsBox<DistributionFunding>>>,
    eliminated: UnboundedReceiver<EliminatedFunding>,
    repo: Arc<Mutex<TRepo>>,
) -> impl Stream<Item = ()> + 'a
where
    TRepo: FundingRepo + 'a,
{
    select_all(vec![
        track_new_funding_boxes(new, Arc::clone(&repo)),
        track_eliminated_funding_boxes(eliminated, repo),
    ])
}

fn track_new_funding_boxes<'a, TRepo>(
    upstream: UnboundedReceiver<Confirmed<AsBox<DistributionFunding>>>,
    repo: Arc<Mutex<TRepo>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TRepo: FundingRepo + 'a,
{
    Box::pin(upstream.then(move |funding| {
        let repo = Arc::clone(&repo);
        async move { repo.lock().put_confirmed(funding).await }
    }))
}

fn track_eliminated_funding_boxes<'a, TRepo>(
    upstream: UnboundedReceiver<EliminatedFunding>,
    repo: Arc<Mutex<TRepo>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TRepo: FundingRepo + 'a,
{
    Box::pin(upstream.then(move |EliminatedFunding(fid)| {
        let repo = Arc::clone(&repo);
        async move { repo.lock().remove(fid).await }
    }))
}
