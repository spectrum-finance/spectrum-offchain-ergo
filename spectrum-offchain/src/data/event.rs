
#[derive(Debug, Clone)]
pub struct Upgrade<T>(pub T);

#[derive(Debug, Clone)]
pub struct UpgradeRollback<T>(pub T);