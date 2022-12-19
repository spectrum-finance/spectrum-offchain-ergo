use ergo_lib::chain::transaction::Transaction;

/// Possible events that can happen with transactions on-chain.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum LedgerTxEvent {
    AppliedTx { timestamp: i64, tx: Transaction },
    UnappliedTx(Transaction),
}
