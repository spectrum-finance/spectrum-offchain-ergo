use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;
use stream_throttle::{ThrottlePool, ThrottleRate, ThrottledStream};

use spectrum_offchain::backlog::Backlog;
use spectrum_offchain::data::order::PendingOrder;

use crate::bundle_repo::BundleRepo;
use crate::data::order::{Compound, Order};
use crate::data::PoolId;

#[async_trait]
pub trait ScheduleStore {
    /// Persist schedule.
    async fn put_schedule(&mut self, schedule: PoolSchedule);
    /// Persist one tick.
    async fn put_tick(&mut self, tick: Tick);
    /// Get closest tick.
    async fn peek(&mut self) -> Option<Tick>;
    /// Mark this tick as temporarily processed.
    async fn check_later(&mut self, tick: Tick);
    /// Remove tick from storage.
    async fn remove(&mut self, tick: Tick);
}

/// Time point when a particular poolshould distribute rewards.
#[derive(Copy, Clone, Debug)]
pub struct Tick {
    pub pool_id: PoolId,
    pub epoch_ix: u32,
    pub timestamp: i64,
}

/// A set of time points when a particular pool should distribute rewards.
pub struct PoolSchedule {
    pub pool_id: PoolId,
    /// unix timestamps in millis.
    pub ticks: Vec<i64>,
}

pub fn distribution_scheduler_stream<'a, TBacklog, TSchedules, TBundles>(
    backlog: Arc<Mutex<TBacklog>>,
    schedules: Arc<Mutex<TSchedules>>,
    bundles: Arc<Mutex<TBundles>>,
    batch_size: usize,
    poll_interval: Duration,
) -> impl Stream<Item = ()> + 'a
where
    TBacklog: Backlog<Order> + 'a,
    TSchedules: ScheduleStore + 'a,
    TBundles: BundleRepo + 'a,
{
    let rate = ThrottleRate::new(5, poll_interval);
    let pool = ThrottlePool::new(rate);
    futures::stream::repeat(()).throttle(pool).then(move |_| {
        let schedules = Arc::clone(&schedules);
        let bundles = Arc::clone(&bundles);
        let backlog = Arc::clone(&backlog);
        async move {
            let mut schedules = schedules.lock();
            if let Some(
                tick @ Tick {
                    pool_id,
                    epoch_ix,
                    timestamp,
                },
            ) = schedules.peek().await
            {
                let ts_now = Utc::now().timestamp();
                if timestamp >= ts_now {
                    let bundles_guard = bundles.lock();
                    let stakers = bundles_guard.select(pool_id, epoch_ix).await;
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
    })
}
