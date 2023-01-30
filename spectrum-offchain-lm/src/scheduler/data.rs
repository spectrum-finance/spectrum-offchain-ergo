use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

use crate::data::pool::{Pool, ProgramConfig};
use crate::data::PoolId;

/// Time point when a particular pool should distribute rewards.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct Tick {
    pub pool_id: PoolId,
    pub epoch_ix: u32,
    pub height: u32,
}

/// A set of time points when a particular pool should distribute rewards.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolSchedule {
    //This part is immutable:
    pub pool_id: PoolId,
    pub epoch_len: u32,
    pub epoch_num: u32,
    pub program_start: u32,
    // Index of the last fully compounded epoch.
    // This index increases as pool progresses:
    pub last_completed_epoch_ix: u32,
}

impl PoolSchedule {
    pub fn next_compounding_at(&self) -> Option<u32> {
        let next_epoch_start =
            self.program_start + self.last_completed_epoch_ix * self.epoch_len + self.epoch_len;
        let program_end = self.program_start + self.epoch_len * self.epoch_num;
        if next_epoch_start < program_end {
            Some(next_epoch_start)
        } else {
            None
        }
    }
}

impl Display for PoolSchedule {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        f.write_str(&*format!(
            "PoolSchedule(pool_id: {}, start: {}, end: {}, step: {}, last_completed_epoch: {}, next_compounding_at: {:?})",
            self.pool_id,
            self.program_start,
            self.program_start + self.epoch_num * self.epoch_len,
            self.epoch_len,
            self.last_completed_epoch_ix,
            self.next_compounding_at()
        ))
    }
}

impl TryFrom<PoolSchedule> for Tick {
    type Error = ();
    fn try_from(sc: PoolSchedule) -> Result<Self, Self::Error> {
        if let Some(h) = sc.next_compounding_at() {
            Ok(Tick {
                pool_id: sc.pool_id,
                epoch_ix: sc.last_completed_epoch_ix + 1,
                height: h,
            })
        } else {
            Err(())
        }
    }
}

impl From<Pool> for PoolSchedule {
    fn from(pool: Pool) -> Self {
        let epochs_left = pool.epochs_left_to_process();
        let ProgramConfig {
            epoch_num,
            epoch_len,
            program_start,
            ..
        } = pool.conf;
        Self {
            pool_id: pool.pool_id,
            epoch_len,
            epoch_num,
            program_start,
            last_completed_epoch_ix: epoch_num - epochs_left,
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
        let schedule = PoolSchedule::from(pool.1.clone());
        assert_eq!(pool.1.clone().conf.program_start, schedule.program_start);
        assert_eq!(pool.1.clone().conf.epoch_len, schedule.epoch_len);
        assert_eq!(pool.1.clone().conf.epoch_num, schedule.epoch_num);
        println!("Schedule: {}", schedule);
    }

    const POOL_JSON: &str = r#"{
        "boxId": "6c70330b8bfe30b5b3ae8b22be6bccf850cda24c3e4cd057d6fea12df3c694bb",
        "value": 1250000,
        "ergoTree": "199d062704000400040204020404040404060406040804080404040204000400040204020601010400040a0500050004040e20a20a53f905f41ebdd71c2c239f270392d0ae0f23f6bd9f3687d166eea745bbf60400040205000402040204060500050005feffffffffffffffff010502050005000402050005000100d820d601b2a5730000d602db63087201d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b27203730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb27202730800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d614998c720c028c720d02d615b27205730a00d6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205730f00d61b7e721a05d61c8c721002d61d998c720f02721cd61e8c720902d61f9d7206721bd6207310d1ededededed93b272027311007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a70193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b172027312959172137313d802d6219c721399721ba273147e721905d622b2a5731500eded937213f07214937221f0721dedededed93cbc272227316938602720e7213b2db6308722273170093860272117221b2db63087222731800e6c67222040893e4c67222050e8c720401958f72137319ededec929a997206721e72079c7e9995907219721a72199a721a731a731b05721f92a39a9a72159c721a7217b27205731c00937214f0721392721d95917219721a731d9c721499721ba2731e7e721905d804d621e4c672010704d62299721a7221d6237e722205d6249999731f721c9c9972127320722395ed917224732191721d7322edededed90722199721973239099721e9c7223721f9a721f7207907ef0998c720802721e069a9d9c99997e721e069d9c7e7206067e7222067e721a0672207e721d067e7224067220937213732493721473257326",
        "assets": [
            {
                "tokenId": "7956620de75192984d639cab2c989269d9a5310ad870ad547426952a9e660699",
                "amount": 1
            },
            {
                "tokenId": "0779ec04f2fae64e87418a1ad917639d4668f78484f45df962b0dec14a2591d2",
                "amount": 299993
            },
            {
                "tokenId": "98da76cecb772029cfec3d53727d5ff37d5875691825fbba743464af0c89ce45",
                "amount": 283146
            },
            {
                "tokenId": "3fdce3da8d364f13bca60998c20660c79c19923f44e141df01349d2e63651e86",
                "amount": 99716855
            },
            {
                "tokenId": "c256908dd9fd477bde350be6a41c0884713a1b1d589357ae731763455ef28c10",
                "amount": 1496035970
            }
        ],
        "creationHeight": 923467,
        "additionalRegisters": {
            "R4": "100490031eeeca70c801",
            "R5": "05becf24",
            "R6": "05d00f",
            "R7": "0402"
        },
        "transactionId": "b5038999043e6ecd617a0a292976fe339d0e4d9ec85296f13610be0c7b16752e",
        "index": 0
    }"#;
}
