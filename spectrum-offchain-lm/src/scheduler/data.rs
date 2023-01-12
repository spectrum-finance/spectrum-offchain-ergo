use std::fmt::{Display, Formatter};

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

impl Display for PoolSchedule {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        f.write_str(&*format!(
            "PoolSchedule(pool_id: {}, start: {:?}, end: {:?})",
            self.pool_id,
            self.ticks.first().cloned(),
            self.ticks.last().cloned()
        ))
    }
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
            ticks: (program_start + epoch_len..program_start + epoch_len + epoch_num * epoch_len)
                .step_by(epoch_len as usize)
                .enumerate()
                .map(|(epoch_ix, h)| (epoch_ix as u32 + 1, h))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;

    use spectrum_offchain::event_sink::handlers::types::TryFromBox;

    use crate::data::pool::Pool;
    use crate::data::AsBox;
    use crate::scheduler::data::PoolSchedule;

    #[test]
    fn schedule_from_pool() {
        let pool_box: ErgoBox = serde_json::from_str(POOL_JSON).unwrap();
        let pool = <AsBox<Pool>>::try_from_box(pool_box).unwrap();
        let schedule = PoolSchedule::from(pool.1);
        assert_eq!(schedule.ticks.first().cloned(), Some((1, 915825)));
        assert_eq!(schedule.ticks.last().cloned(), Some((10, 924825)));
    }

    const POOL_JSON: &str = r#"{
        "boxId": "6a928dad5999caeaf08396f9a40c37b1b09ce2b972df457f4afacb61ff69009a",
        "value": 1250000,
        "ergoTree": "19e9041f04000402040204040404040604060408040804040402040004000402040204000400040a0500040204020500050004020402040605000500040205000500d81bd601b2a5730000d602db63087201d603db6308a7d604e4c6a70410d605e4c6a70505d606e4c6a70605d607b27202730100d608b27203730200d609b27202730300d60ab27203730400d60bb27202730500d60cb27203730600d60db27202730700d60eb27203730800d60f8c720a02d610998c720902720fd6118c720802d612b27204730900d6139a99a37212730ad614b27204730b00d6159d72137214d61695919e72137214730c9a7215730d7215d617b27204730e00d6187e721705d6199d72057218d61a998c720b028c720c02d61b998c720d028c720e02d1ededededed93b27202730f00b27203731000ededed93e4c672010410720493e4c672010505720593e4c6720106057206928cc77201018cc7a70193c27201c2a7ededed938c7207018c720801938c7209018c720a01938c720b018c720c01938c720d018c720e0193b172027311959172107312eded929a997205721172069c7e9995907216721772169a721773137314057219937210f0721a939c7210997218a273157e721605f0721b958f72107316ededec929a997205721172069c7e9995907216721772169a72177317731805721992a39a9a72129c72177214b2720473190093721af0721092721b959172167217731a9c721a997218a2731b7e721605d801d61ce4c672010704edededed90721c997216731c909972119c7e997217721c0572199a7219720693f0998c72070272117d9d9c7e7219067e721b067e720f0605937210731d93721a731e",
        "assets": [
            {
                "tokenId": "5c7b7988f34bfb13059c447c648c23c36d121228588fa9ea68b9943b0333ea4c",
                "amount": 1
            },
            {
                "tokenId": "0779ec04f2fae64e87418a1ad917639d4668f78484f45df962b0dec14a2591d2",
                "amount": 10000
            },
            {
                "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                "amount": 100
            },
            {
                "tokenId": "3fdce3da8d364f13bca60998c20660c79c19923f44e141df01349d2e63651e86",
                "amount": 10000000
            },
            {
                "tokenId": "c256908dd9fd477bde350be6a41c0884713a1b1d589357ae731763455ef28c10",
                "amount": 99999000
            }
        ],
        "creationHeight": 913825,
        "additionalRegisters": {
            "R4": "1004d00f1492d66fd00f",
            "R5": "05a09c01",
            "R6": "05d00f"
        },
        "transactionId": "ceb08428111f9d4dbcba3b0de024326af032e52f37ea189b8bd3affbcc40a3a6",
        "index": 0
    }"#;
}
