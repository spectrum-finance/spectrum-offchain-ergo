use async_trait::async_trait;

use crate::data::pool::ProgramConfig;
use crate::data::PoolId;

/// Registry of all LM programs known in the network.
#[async_trait]
pub trait ProgramRepo {
    /// Persist the given `ProgramConfig` unless it is already present in db.
    async fn put(&self, pool_id: PoolId, conf: ProgramConfig);
    /// Get `ProgramConfig` corresponding to the given `PoolId`.
    async fn get(&self, pool_id: PoolId) -> Option<ProgramConfig>;
}
