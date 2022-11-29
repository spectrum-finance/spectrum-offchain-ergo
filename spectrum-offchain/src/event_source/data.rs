use ergo_lib::chain::transaction::Transaction;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TxEvent {
    AppliedTx(Transaction),
    UnappliedTx(Transaction),
}