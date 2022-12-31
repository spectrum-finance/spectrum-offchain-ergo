use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::{Stream, StreamExt};
use stream_throttle::{ThrottlePool, ThrottleRate, ThrottledStream};
use tokio::sync::Mutex;

use spectrum_offchain::backlog::Backlog;
use spectrum_offchain::data::order::PendingOrder;
use spectrum_offchain::network::ErgoNetwork;

use crate::bundle::BundleRepo;
use crate::data::order::{Compound, Order};
use crate::scheduler::data::Tick;
use crate::scheduler::ScheduleRepo;

pub fn distribution_stream<'a, TBacklog, TSchedules, TBundles, TNetwork>(
    backlog: Arc<Mutex<TBacklog>>,
    schedules: Arc<Mutex<TSchedules>>,
    bundles: Arc<Mutex<TBundles>>,
    network: TNetwork,
    batch_size: usize,
    poll_interval: Duration,
) -> impl Stream<Item = ()> + 'a
where
    TBacklog: Backlog<Order> + 'a,
    TSchedules: ScheduleRepo + 'a,
    TBundles: BundleRepo + 'a,
    TNetwork: ErgoNetwork + Clone + 'a,
{
    let rate = ThrottleRate::new(5, poll_interval);
    let pool = ThrottlePool::new(rate);
    Box::pin(futures::stream::repeat(()).throttle(pool).then(move |_| {
        let schedules = Arc::clone(&schedules);
        let bundles = Arc::clone(&bundles);
        let backlog = Arc::clone(&backlog);
        let network = network.clone();
        async move {
            let mut schedules = schedules.lock().await;
            if let Some(
                tick @ Tick {
                    pool_id,
                    epoch_ix,
                    height,
                },
            ) = schedules.peek().await
            {
                let height_now = network.get_height().await;
                if height >= height_now {
                    let stakers = bundles.lock().await.select(pool_id, epoch_ix).await;
                    if stakers.is_empty() {
                        schedules.remove(tick).await;
                    } else {
                        let orders =
                            stakers
                                .chunks(batch_size)
                                .into_iter()
                                .enumerate()
                                .map(|(queue_ix, xs)| Compound {
                                    pool_id,
                                    epoch_ix,
                                    queue_ix,
                                    stakers: Vec::from(xs),
                                });
                        let ts_now = Utc::now().timestamp();
                        let mut backlog = backlog.lock().await;
                        for order in orders {
                            backlog
                                .put(PendingOrder {
                                    order: Order::Compound(order),
                                    timestamp: ts_now,
                                })
                                .await
                        }
                        // todo: DEV-601: Check later instead of just removing.
                        schedules.remove(tick).await;
                    }
                }
            }
        }
    }))
}
