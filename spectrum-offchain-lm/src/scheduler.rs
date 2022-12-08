use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use chrono::Utc;
use futures::{stream, Stream, StreamExt};
use parking_lot::Mutex;
use pin_project::pin_project;

use spectrum_offchain::backlog::Backlog;

use crate::data::bundle::StakingBundle;
use crate::data::order::{Compound, Order};
use crate::data::{BundleId, PoolId};

pub struct ScheduledBundle {
    pub epoch_ix: u32,
    bundle: StakingBundle,
}

#[async_trait]
pub trait DistributionHistory {
    /// Mark given stakers as processed for the given epoch.
    async fn processed(&mut self, epoch_ix: u32, stakers: Vec<BundleId>);
    /// Un-mark given stakers as processed for the given epoch.
    async fn revert(&mut self, epoch_ix: u32, stakers: Vec<BundleId>);
}

#[async_trait]
pub trait ScheduleStore {
    /// Persist schedule.
    async fn put_schedule(&mut self, schedule: PoolSchedule);
    /// Persist one tick.
    async fn put_tick(&mut self, tick: Tick);
    /// Get closest tick.
    async fn pop(&mut self) -> Option<Tick>;
}

/// Time point when a particular poolshould distribute rewards.
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

#[async_trait]
pub trait BundleRegistry {
    async fn register(&mut self, bundle: StakingBundle);
    async fn derigister(&mut self, bundle_id: BundleId);
    async fn select_by_pool(&self, pool_id: PoolId) -> Vec<BundleId>;
}

#[pin_project]
pub struct RewardDistributionScheduler<TBacklog, TSchedules, TBundles> {
    #[pin]
    backlog: TBacklog,
    #[pin]
    schedules: TSchedules,
    #[pin]
    bundles: TBundles,
}

impl<TBacklog, TSchedules, TBundles> Stream for RewardDistributionScheduler<TBacklog, TSchedules, TBundles>
where
    TBacklog: Backlog<Order>,
    TSchedules: ScheduleStore + Send,
    TBundles: BundleRegistry,
{
    type Item = Compound;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.project();
        let mut pop_tick_fut = this.schedules.pop();
        loop {
            match pop_tick_fut.as_mut().poll(cx) {
                Poll::Ready(Some(tick)) => {}
                Poll::Ready(None) => break,
                Poll::Pending => {}
            }
        }
        Poll::Pending
    }
}

// pub fn reward_distribution_stream<TBacklog, TStore>(
//     backlog: TBacklog,
//     schedules: TStore,
// ) -> impl Stream<Item = Compound>
// where
//     TBacklog: Backlog<Order>,
//     TStore: ScheduleStore,
// {
//     let backlog = Arc::new(Mutex::new(backlog));
//     let schedules = Arc::new(Mutex::new(schedules));
//     stream::iter(0..).then(move |_| {
//         let schedules = Arc::clone(&schedules);
//         async move {
//             let mut schedules_guard = schedules.lock();
//             if let Some(Tick { pool_id, timestamp }) = schedules_guard.pop().await {
//                 let ts_now = Utc::now().timestamp();
//                 if timestamp >= ts_now {
//
//                 }
//             }
//         }
//     })
// }
