use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::{Stream, StreamExt};
use futures::channel::mpsc::UnboundedReceiver;
use futures::stream::select_all;
use parking_lot::Mutex;
use stream_throttle::{ThrottledStream, ThrottlePool, ThrottleRate};

use spectrum_offchain::backlog::Backlog;
use spectrum_offchain::data::order::PendingOrder;
use spectrum_offchain::data::unique_entity::{Confirmed, Upgrade};
use spectrum_offchain::network::ErgoNetwork;

use crate::bundle::BundleRepo;
use crate::data::order::{Compound, Order};
use crate::data::pool::Pool;
use crate::scheduler::data::{PoolSchedule, Tick};
use crate::scheduler::ScheduleRepo;

pub fn run_distribution_scheduler<'a, TBacklog, TSchedules, TBundles, TNetwork>(
    confirmed_pools: UnboundedReceiver<Upgrade<Confirmed<Pool>>>,
    backlog: Arc<Mutex<TBacklog>>,
    schedules: Arc<Mutex<TSchedules>>,
    bundles: Arc<Mutex<TBundles>>,
    network: TNetwork,
    batch_size: usize,
    poll_interval: Duration,
) -> impl Stream<Item = ()> + 'a
where
    TSchedules: ScheduleRepo + 'a,
    TBacklog: Backlog<Order> + 'a,
    TSchedules: ScheduleRepo + 'a,
    TBundles: BundleRepo + 'a,
    TNetwork: ErgoNetwork + Clone + 'a,
{
    select_all(vec![
        distribution_stream(
            backlog,
            Arc::clone(&schedules),
            bundles,
            network,
            batch_size,
            poll_interval,
        ),
        track_pools(confirmed_pools, schedules),
    ])
}

fn distribution_stream<'a, TBacklog, TSchedules, TBundles, TNetwork>(
    backlog: Arc<Mutex<TBacklog>>,
    schedules: Arc<Mutex<TSchedules>>,
    bundles: Arc<Mutex<TBundles>>,
    network: TNetwork,
    batch_size: usize,
    poll_interval: Duration,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
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
            let mut schedules = schedules.lock();
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
                    let stakers = bundles.lock().select(pool_id, epoch_ix).await;
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
                        let mut backlog = backlog.lock();
                        for order in orders {
                            backlog
                                .put(PendingOrder {
                                    order: Order::Compound(order),
                                    timestamp: ts_now,
                                })
                                .await
                        }
                        schedules.check_later(tick).await;
                    }
                }
            }
        }
    }))
}

fn track_pools<'a, TSchedules>(
    upstream: UnboundedReceiver<Upgrade<Confirmed<Pool>>>,
    schedules: Arc<Mutex<TSchedules>>,
) -> Pin<Box<dyn Stream<Item = ()> + 'a>>
where
    TSchedules: ScheduleRepo + 'a,
{
    Box::pin(upstream.then(move |Upgrade(Confirmed(pool))| {
        let schedules = Arc::clone(&schedules);
        async move {
            let mut schedules = schedules.lock();
            if !schedules.exists(pool.pool_id).await {
                schedules.put_schedule(PoolSchedule::from(pool)).await
            }
        }
    }))
}
