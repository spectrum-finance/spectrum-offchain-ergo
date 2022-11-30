use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;

/// Used to convert `ErgoBox` to domain entity `TEntity`.
pub trait TryFromBox<TEntity> {
    /// Try to extract some domain entity `TEntity` from given `ErgoBox`.
    fn try_from(&self, bx: ErgoBox) -> Option<TEntity>;
}
