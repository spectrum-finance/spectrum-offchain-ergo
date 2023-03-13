use super::OnChainOrderId;

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub enum OrderUpdate<TOrd: OnChainOrderId> {
    NewOrder(PendingOrder<TOrd>),
    OrderEliminated(TOrd::TOrderId),
}

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub struct PendingOrder<TOrd> {
    pub order: TOrd,
    pub timestamp: i64,
}

impl<TOrd> From<ProgressingOrder<TOrd>> for PendingOrder<TOrd> {
    fn from(po: ProgressingOrder<TOrd>) -> Self {
        Self {
            order: po.order,
            timestamp: po.timestamp,
        }
    }
}

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub struct SuspendedOrder<TOrd> {
    pub order: TOrd,
    pub timestamp: i64,
}

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub struct ProgressingOrder<TOrd> {
    pub order: TOrd,
    pub timestamp: i64,
}
