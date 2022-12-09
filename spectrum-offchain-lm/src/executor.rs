use std::sync::Arc;

use async_trait::async_trait;
use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use futures::{stream, StreamExt};
use itertools::{EitherOrBoth, Itertools};
use log::{error, warn};
use parking_lot::Mutex;

use spectrum_offchain::backlog::Backlog;
use spectrum_offchain::box_resolver::BoxResolver;
use spectrum_offchain::data::unique_entity::{Predicted, Traced};
use spectrum_offchain::data::{Has, OnChainEntity, OnChainOrder};
use spectrum_offchain::executor::Executor;
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::network::ErgoNetwork;

use crate::bundle_repo::BundleRepo;
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
    backlog: Arc<Mutex<TBacklog>>,
    pool_resolver: Arc<Mutex<TPoolResolver>>,
    bundle_resolver: Arc<Mutex<TBundleResolver>>,
    executor_prop: ErgoTree,
}

impl<TNetwork, TBacklog, TPoolResolver, TBundleResolver>
    OrderExecutor<TNetwork, TBacklog, TPoolResolver, TBundleResolver>
{
    async fn get_context(&self, first_input_id: BoxId) -> ExecutionContext {
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
    TBacklog: Backlog<Order>,
    TPoolResolver: BoxResolver<AsBox<Pool>>,
    TBundleResolver: BundleRepo,
{
    async fn execute_next(&mut self) {
        let mut backlog = self.backlog.lock();
        if let Some(ord) = backlog.try_pop().await {
            let entity_id = ord.get_entity_ref();
            let mut pool_resolver = self.pool_resolver.lock();
            if let Some(pool) = pool_resolver.get(entity_id).await {
                let bundle_ids = ord.get::<Vec<BundleId>>();
                let bundle_resolver = Arc::clone(&self.bundle_resolver);
                let bundles = stream::iter(bundle_ids.iter())
                    .scan((), move |_, bundle_id| {
                        let bundle_resolver = Arc::clone(&bundle_resolver);
                        async move { bundle_resolver.lock().get(*bundle_id).await }
                    })
                    .collect::<Vec<_>>()
                    .await;
                let ctx = self.get_context(pool.box_id()).await;
                let run_result = match (ord.clone(), bundles.first().cloned()) {
                    (Order::Deposit(deposit), _) => deposit
                        .try_run(pool.clone(), (), ctx)
                        .map(|(tx, next_pool, bundle)| (tx, next_pool, vec![bundle]))
                        .map_err(|err| err.map(Order::Deposit)),
                    (Order::Redeem(redeem), Some(bundle)) => redeem
                        .try_run(pool.clone(), bundle, ctx)
                        .map(|(tx, next_pool, _)| (tx, next_pool, Vec::new()))
                        .map_err(|err| err.map(Order::Redeem)),
                    (Order::Compound(compound), _) if !bundles.is_empty() => compound
                        .try_run(pool.clone(), bundles.clone(), ctx)
                        .map_err(|err| err.map(Order::Compound)),
                    (Order::Compound(_), _) => Err(RunOrderError::Fatal(
                        String::from("No bundles found for Compound"),
                        ord.clone(),
                    )),
                    (Order::Redeem(redeem), None) => {
                        error!("Bunle not found for Redeem [{:?}]", redeem.get_self_ref());
                        return;
                    }
                };
                match run_result {
                    Ok((tx, next_pool, next_bundles)) => {
                        if let Err(client_err) = self.network.submit_tx(tx).await {
                            // Note, here `submit_tx(tx)` can fail not only bc pool state is consumed,
                            // but also bc bundle is consumed, what is less possible though. That's why
                            // we just invalidate pool.
                            // todo: In the future more precise error handling may be possible if we
                            // todo: implement a way to find out which input failed exactly.
                            warn!("Execution failed while submitting tx due to {}", client_err);
                            pool_resolver
                                .invalidate(pool.get_self_ref(), pool.get_self_state_ref())
                                .await;
                            backlog.recharge(ord).await; // Return order to backlog
                        } else {
                            pool_resolver
                                .put(Traced {
                                    state: next_pool,
                                    prev_state_id: Some(pool.get_self_state_ref()),
                                })
                                .await;
                            let bundle_resolver = self.bundle_resolver.lock();
                            for bundle_st in next_bundles.into_iter().zip_longest(bundles) {
                                match bundle_st {
                                    EitherOrBoth::Both(next_bundle, prev_bundle) => {
                                        bundle_resolver
                                            .put_predicated(Traced {
                                                state: next_bundle,
                                                prev_state_id: Some(prev_bundle.1.get_self_state_ref()),
                                            })
                                            .await
                                    }
                                    EitherOrBoth::Left(next_bundle) => {
                                        bundle_resolver
                                            .put_predicated(Traced {
                                                state: next_bundle,
                                                prev_state_id: None,
                                            })
                                            .await
                                    }
                                    EitherOrBoth::Right(_) => {}
                                }
                            }
                        }
                    }
                    Err(RunOrderError::NonFatal(err, ord)) => {
                        warn!("Order suspended due to non-fatal error {}", err);
                        backlog.suspend(ord).await;
                    }
                    Err(RunOrderError::Fatal(err, ord)) => {
                        warn!("Order dropped due to fatal error {}", err);
                        backlog.remove(ord.get_self_ref()).await;
                    }
                }
            }
        }
    }
}
