use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, ErgoBoxCandidate};

/// Used to convert `ErgoBox` to domain entity.
pub trait TryFromBox: Sized {
    /// Try to extract some domain entity from given `ErgoBox`.
    fn try_from_box(bx: ErgoBox) -> Option<Self>;
}

/// Used to convert some domain entity to `ErgoBoxCandidate`.
pub trait IntoBoxCandidate {
    fn into_candidate(self, height: u32) -> ErgoBoxCandidate;
}
