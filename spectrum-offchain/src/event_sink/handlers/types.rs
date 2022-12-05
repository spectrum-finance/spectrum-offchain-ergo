use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;

/// Used to convert `ErgoBox` to domain entity.
pub trait TryFromBox: Sized {
    /// Try to extract some domain entity from given `ErgoBox`.
    fn try_from_box(bx: ErgoBox) -> Option<Self>;
}

/// Used to convert some domain entity to `ErgoBox`.
pub trait IntoBox {
    fn into_box(self) -> ErgoBox;
}
