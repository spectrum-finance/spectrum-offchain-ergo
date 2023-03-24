use std::sync::{Arc, Once};
use std::time::Duration;

use chrono::Utc;
use futures::{Stream, StreamExt};
use log::info;
use spectrum_offchain::data::OnChainOrder;
use stream_throttle::{ThrottlePool, ThrottleRate, ThrottledStream};
use tokio::sync::Mutex;

use spectrum_offchain::backlog::Backlog;
use spectrum_offchain::data::order::PendingOrder;
use spectrum_offchain::network::ErgoNetwork;

use crate::bundle::BundleRepo;
use crate::data::order::{Compound, Order};
use crate::scheduler::data::Tick;
use crate::scheduler::ScheduleRepo;

const TICK_SUSPENSION_DURATION: i64 = 60 * 30;

pub fn distribution_stream<'a, TBacklog, TSchedules, TBundles, TNetwork>(
    backlog: Arc<Mutex<TBacklog>>,
    schedules: Arc<Mutex<TSchedules>>,
    bundles: Arc<Mutex<TBundles>>,
    network: &'a TNetwork,
    batch_size: usize,
    poll_interval: Duration,
    tip_reached: &'a Once,
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
            if tip_reached.is_completed() {
                let peek_result = {
                    let mut schedules = schedules.lock().await;
                    schedules.peek().await
                };
                if let Some(
                    tick @ Tick {
                        pool_id,
                        epoch_ix,
                        height,
                    },
                ) = peek_result
                {
                    info!(target: "scheduler", "Checking schedule of pool [{}]", pool_id);
                    let height_now = network.get_height().await;
                    if height <= height_now {
                        info!(target: "scheduler", "Processing epoch [{}] of pool [{}]", epoch_ix, pool_id);
                        let mut stakers = bundles.lock().await.select(pool_id, epoch_ix).await;
                        stakers.sort();
                        if stakers.is_empty() {
                            info!(
                                target: "scheduler",
                                "No more stakers left in epoch [{}] of pool [{}]",
                                epoch_ix, pool_id
                            );

                            // No stakers means that the compounding orders for this epoch are
                            // complete, and so they should be removed from the backlog.
                            let mut backlog = backlog.lock().await;
                            let compound_orders = backlog
                                .find_orders(move |ord| {
                                    if let Order::Compound(c) = ord {
                                        c.epoch_ix == epoch_ix && c.pool_id == pool_id
                                    } else {
                                        false
                                    }
                                })
                                .await;
                            for c in compound_orders {
                                backlog.remove(c.get_self_ref()).await;
                            }

                            {
                                // Note that our implementation of `ScheduleRepo::remove(..)`
                                // applies only to deferred ticks. Now this tick has no stakers and
                                // thus will never be deferred. It must be removed by
                                // `ScheduleRepo` though, so as a workaround we simply defer the
                                // tick, then remove.
                                let ts_now = Utc::now().timestamp();
                                let mut schedules = schedules.lock().await;
                                schedules.defer(tick, ts_now + TICK_SUSPENSION_DURATION).await;
                                schedules.remove(tick).await;
                            }
                        } else {
                            let orders =
                                stakers
                                    .chunks(batch_size)
                                    .enumerate()
                                    .map(|(queue_ix, xs)| Compound {
                                        pool_id,
                                        epoch_ix,
                                        queue_ix,
                                        stakers: Vec::from(xs),
                                    });
                            info!(
                                target: "scheduler",
                                "# stakers left in epoch [{}] of pool [{}]: {}",
                                epoch_ix, pool_id, stakers.len(),
                            );
                            let ts_now = Utc::now().timestamp();
                            for order in orders {
                                let mut backlog = backlog.lock().await;
                                backlog
                                    .put(PendingOrder {
                                        order: Order::Compound(order),
                                        timestamp: ts_now,
                                    })
                                    .await;
                            }
                            let mut schedules = schedules.lock().await;
                            schedules.defer(tick, ts_now + TICK_SUSPENSION_DURATION).await;
                        }
                    }
                }
            }
        }
    }))
}
