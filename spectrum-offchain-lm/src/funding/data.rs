use ergo_lib::ergotree_ir::chain::address::Address;
use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, ErgoBoxCandidate, NonMandatoryRegisters};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;

use spectrum_offchain::event_sink::handlers::types::{IntoBoxCandidate, TryFromBox, TryFromBoxCtx};

use crate::data::FundingId;
use crate::ergo::NanoErg;

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct DistributionFundingProto {
    pub prop: ErgoTree,
    pub erg_value: NanoErg,
}

impl DistributionFundingProto {
    pub fn complete(self, id: FundingId) -> DistributionFunding {
        DistributionFunding {
            id,
            prop: self.prop,
            erg_value: self.erg_value,
        }
    }
}

impl IntoBoxCandidate for DistributionFundingProto {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        ErgoBoxCandidate {
            value: self.erg_value.into(),
            ergo_tree: self.prop,
            tokens: None,
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: height,
        }
    }
}

impl From<DistributionFunding> for DistributionFundingProto {
    fn from(df: DistributionFunding) -> Self {
        Self {
            prop: df.prop,
            erg_value: df.erg_value,
        }
    }
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct DistributionFunding {
    pub id: FundingId,
    pub prop: ErgoTree,
    pub erg_value: NanoErg,
}

/// Discarded on-chain entity.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct EliminatedFunding(pub FundingId);

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct ExecutorWallet(Address);

impl TryFromBoxCtx<ExecutorWallet> for DistributionFunding {
    fn try_from_box(bx: ErgoBox, ctx: ExecutorWallet) -> Option<Self> {
        todo!()
    }
}

impl IntoBoxCandidate for DistributionFunding {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate {
        DistributionFundingProto::from(self).into_candidate(height)
    }
}
