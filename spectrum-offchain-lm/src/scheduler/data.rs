use serde::{Deserialize, Serialize};

use crate::data::pool::{Pool, ProgramConfig};
use crate::data::PoolId;

/// Time point when a particular poolshould distribute rewards.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct Tick {
    pub pool_id: PoolId,
    pub epoch_ix: u32,
    pub height: u32,
}

/// A set of time points when a particular pool should distribute rewards.
#[derive(Clone, Debug)]
pub struct PoolSchedule {
    pub pool_id: PoolId,
    /// Collection of pairs (epoch_ix, height).
    pub ticks: Vec<(u32, u32)>,
}

impl From<PoolSchedule> for Vec<Tick> {
    fn from(ps: PoolSchedule) -> Self {
        ps.ticks
            .into_iter()
            .map(|(epoch_ix, height)| Tick {
                pool_id: ps.pool_id,
                epoch_ix,
                height,
            })
            .collect()
    }
}

impl From<Pool> for PoolSchedule {
    fn from(pool: Pool) -> Self {
        let ProgramConfig {
            epoch_num,
            epoch_len,
            program_start,
            ..
        } = pool.conf;
        Self {
            pool_id: pool.pool_id,
            ticks: (program_start..program_start + epoch_num * epoch_len)
                .step_by(epoch_len as usize)
                .enumerate()
                .map(|(epoch_ix, h)| (epoch_ix as u32, h))
                .collect(),
        }
    }
}
