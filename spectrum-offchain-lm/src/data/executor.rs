use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBoxCandidate, NonMandatoryRegisters};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::event_sink::handlers::types::IntoBoxCandidate;

use crate::ergo::NanoErg;

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct ExecutorOutput {
    pub executor_prop: ErgoTree,
    pub erg_value: NanoErg,
}

impl IntoBoxCandidate for ExecutorOutput {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        ErgoBoxCandidate {
            value: self.erg_value.into(),
            ergo_tree: self.executor_prop,
            tokens: None,
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: height,
        }
    }
}
