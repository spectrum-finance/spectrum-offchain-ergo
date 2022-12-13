use async_trait::async_trait;

use crate::data::PoolId;
use crate::scheduler::data::{PoolSchedule, Tick};

pub mod data;
pub mod process;

#[async_trait]
pub trait ScheduleRepo {
    /// Persist schedule.
    async fn put_schedule(&mut self, schedule: PoolSchedule);
    /// Check whether a schedule for the given pool exists.
    async fn exists(&self, pool_id: PoolId) -> bool;
    /// Persist one tick.
    async fn put_tick(&mut self, tick: Tick);
    /// Get closest tick.
    async fn peek(&mut self) -> Option<Tick>;
    /// Mark this tick as temporarily processed.
    async fn check_later(&mut self, tick: Tick);
    /// Remove tick from storage.
    async fn remove(&mut self, tick: Tick);
}
