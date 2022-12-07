use async_trait::async_trait;
use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use log::{error, warn};

use spectrum_offchain::backlog::Backlog;
use spectrum_offchain::box_resolver::BoxResolver;
use spectrum_offchain::data::unique_entity::{Predicted, Traced};
use spectrum_offchain::data::{Has, OnChainEntity, OnChainOrder};
use spectrum_offchain::executor::Executor;
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::network::ErgoNetwork;

use crate::data::bundle::StakingBundle;
use crate::data::context::ExecutionContext;
use crate::data::order::Order;
use crate::data::pool::Pool;
use crate::data::{AsBox, BundleId};

pub trait ConsumeBundle {
    type TBundleIn;
}

pub trait ProduceBundle {
    type TBundleOut;
}

pub trait RunOrder: OnChainOrder + ConsumeBundle + ProduceBundle + Sized {
    /// Try to run the given `Order` against the given `Pool`.
    /// Returns transaction, next state of the pool and optionally staking bundle in the case of success.
    /// Returns `RunOrderError<TOrd>` otherwise.
    fn try_run(
        self,
        pool: AsBox<Pool>,
        bundle: Self::TBundleIn,
        ctx: ExecutionContext,
    ) -> Result<(Transaction, Predicted<AsBox<Pool>>, Self::TBundleOut), RunOrderError<Self>>;
}

pub struct OrderExecutor<TNetwork, TBacklog, TPoolResolver, TBundleResolver> {
    network: TNetwork,
    backlog: TBacklog,
    pool_resolver: TPoolResolver,
    bundle_resolver: TBundleResolver,
    executor_prop: ErgoTree,
}

impl<TNetwork, TBacklog, TPoolResolver, TBundleResolver>
    OrderExecutor<TNetwork, TBacklog, TPoolResolver, TBundleResolver>
{
    async fn get_context(&mut self, first_input_id: BoxId) -> ExecutionContext {
        ExecutionContext {
            height: 0, // todo
            mintable_token_id: first_input_id.into(),
            executor_prop: self.executor_prop.clone(),
        }
    }
}

#[async_trait(?Send)]
impl<TNetwork, TBacklog, TPoolResolver, TBundleResolver> Executor
    for OrderExecutor<TNetwork, TBacklog, TPoolResolver, TBundleResolver>
where
    TNetwork: ErgoNetwork,
    TBacklog: Backlog<AsBox<Order>>,
    TPoolResolver: BoxResolver<AsBox<Pool>>,
    TBundleResolver: BoxResolver<AsBox<StakingBundle>>,
{
    async fn execute_next(&mut self) {
        if let Some(ord) = self.backlog.try_pop().await {
            let entity_id = ord.get_entity_ref();
            if let Some(pool) = self.pool_resolver.get(entity_id).await {
                let bundle = if let Some(bundle_id) = ord.get::<Option<BundleId>>() {
                    self.bundle_resolver.get(bundle_id).await
                } else {
                    None
                };
                let ctx = self.get_context(pool.box_id()).await;
                let run_result = match (ord.clone(), bundle.clone()) {
                    (AsBox(input, Order::Deposit(deposit)), _) => AsBox(input, deposit)
                        .try_run(pool.clone(), (), ctx)
                        .map(|(tx, next_pool, bundle)| (tx, next_pool, Some(bundle)))
                        .map_err(|err| err.map(|as_box| as_box.map(Order::Deposit))),
                    (AsBox(input, Order::Redeem(redeem)), Some(bundle)) => AsBox(input, redeem)
                        .try_run(pool.clone(), bundle, ctx)
                        .map(|(tx, next_pool, _)| (tx, next_pool, None))
                        .map_err(|err| err.map(|as_box| as_box.map(Order::Redeem))),
                    (AsBox(_, Order::Redeem(redeem)), None) => {
                        error!("Bunle not found for Redeem [{:?}]", redeem.get_self_ref());
                        return;
                    }
                };
                match run_result {
                    Ok((tx, next_pool_state, next_bundle_state)) => {
                        if let Err(client_err) = self.network.submit_tx(tx).await {
                            // Note, here `submit_tx(tx)` can fail not only bc pool state is consumed,
                            // but also bc bundle is consumed, what is less possible though. That's why
                            // we just invalidate pool.
                            // todo: In the future more precise error handling may be possible if we
                            // todo: implement a way to find out which input failed exactly.
                            warn!("Execution failed while submitting tx due to {}", client_err);
                            self.pool_resolver
                                .invalidate(pool.get_self_ref(), pool.get_self_state_ref())
                                .await;
                            self.backlog.recharge(ord).await; // Return order to backlog
                        } else {
                            self.pool_resolver
                                .put(Traced {
                                    state: next_pool_state,
                                    prev_state_id: Some(pool.get_self_state_ref()),
                                })
                                .await;
                            if let Some(next_bundle_state) = next_bundle_state {
                                self.bundle_resolver
                                    .put(Traced {
                                        state: next_bundle_state,
                                        prev_state_id: bundle.map(|b| b.get_self_state_ref()),
                                    })
                                    .await;
                            }
                        }
                    }
                    Err(RunOrderError::NonFatal(err, ord)) => {
                        warn!("Order suspended due to non-fatal error {}", err);
                        self.backlog.suspend(ord).await;
                    }
                    Err(RunOrderError::Fatal(err, ord)) => {
                        warn!("Order dropped due to fatal error {}", err);
                        self.backlog.remove(ord.get_self_ref()).await;
                    }
                }
            }
        }
    }
}
