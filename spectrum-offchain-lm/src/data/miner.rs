use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBoxCandidate, NonMandatoryRegisters};
use ergo_lib::wallet::miner_fee::MINERS_FEE_ADDRESS;

use spectrum_offchain::event_sink::handlers::types::IntoBoxCandidate;

use crate::ergo::NanoErg;

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct MinerOutput {
    pub erg_value: NanoErg,
}

impl IntoBoxCandidate for MinerOutput {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        ErgoBoxCandidate {
            value: self.erg_value.into(),
            ergo_tree: MINERS_FEE_ADDRESS.script().unwrap(),
            tokens: None,
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: height,
        }
    }
}
