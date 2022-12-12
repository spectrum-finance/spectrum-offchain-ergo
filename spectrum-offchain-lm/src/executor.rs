use std::cell::Cell;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use futures::{stream, StreamExt};
use itertools::{EitherOrBoth, Itertools};
use log::{error, warn};
use nonempty::NonEmpty;
use parking_lot::Mutex;

use spectrum_offchain::backlog::Backlog;
use spectrum_offchain::box_resolver::BoxResolver;
use spectrum_offchain::data::unique_entity::{Predicted, Traced};
use spectrum_offchain::data::{Has, OnChainEntity, OnChainOrder};
use spectrum_offchain::executor::Executor;
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::network::ErgoNetwork;
use spectrum_offchain::transaction::TransactionCandidate;

use crate::bundle::BundleRepo;
use crate::data::context::ExecutionContext;
use crate::data::order::Order;
use crate::data::pool::Pool;
use crate::data::{AsBox, BundleId};
use crate::funding::FundingRepo;
use crate::prover::SigmaProver;

/// Defines additional inputs required to execute the operation.
pub trait ConsumeExtra {
    type TExtraIn;
}

/// Defines additional outputs resulted from execution of the operation.
pub trait ProduceExtra {
    type TExtraOut;
}

pub trait RunOrder: OnChainOrder + ConsumeExtra + ProduceExtra + Sized {
    /// Try to run the given `Order` against the given `Pool`.
    /// Returns transaction, next state of the pool and optionally staking bundle in the case of success.
    /// Returns `RunOrderError<TOrd>` otherwise.
    fn try_run(
        self,
        pool: AsBox<Pool>,
        extra: Self::TExtraIn,
        ctx: ExecutionContext,
    ) -> Result<(TransactionCandidate, Predicted<AsBox<Pool>>, Self::TExtraOut), RunOrderError<Self>>;
}

pub struct OrderExecutor<TNetwork, TBacklog, TPoolResolver, TBundleResolver, TFunding, TProver> {
    network: TNetwork,
    backlog: Arc<Mutex<TBacklog>>,
    pool_repo: Arc<Mutex<TPoolResolver>>,
    bundle_repo: Arc<Mutex<TBundleResolver>>,
    funding_repo: Arc<Mutex<TFunding>>,
    prover: TProver,
    executor_prop: ErgoTree,
    context_cache: Cell<(u32, i64)>,
}

const CTX_TTL_SECS: i64 = 30;

impl<TNetwork, TBacklog, TPools, TBundles, TFunding, TProver>
    OrderExecutor<TNetwork, TBacklog, TPools, TBundles, TFunding, TProver>
where
    TNetwork: ErgoNetwork,
{
    async fn make_context(&self, first_input_id: BoxId) -> ExecutionContext {
        let (cached_height, ts) = self.context_cache.get();
        let ts_now = Utc::now().timestamp();
        let height = if ts_now - ts >= CTX_TTL_SECS {
            let h = self.network.get_height().await;
            self.context_cache.set((h, ts_now));
            h
        } else {
            cached_height
        };
        ExecutionContext {
            height,
            mintable_token_id: first_input_id.into(),
            executor_prop: self.executor_prop.clone(),
        }
    }
}

#[async_trait(?Send)]
impl<TNetwork, TBacklog, TPools, TBundles, TFunding, TProver> Executor
    for OrderExecutor<TNetwork, TBacklog, TPools, TBundles, TFunding, TProver>
where
    TNetwork: ErgoNetwork,
    TBacklog: Backlog<Order>,
    TPools: BoxResolver<AsBox<Pool>>,
    TBundles: BundleRepo,
    TFunding: FundingRepo,
    TProver: SigmaProver,
{
    async fn execute_next(&mut self) {
        if let Some(ord) = self.backlog.lock().try_pop().await {
            let entity_id = ord.get_entity_ref();
            let mut pool_resolver = self.pool_repo.lock();
            if let Some(pool) = pool_resolver.get(entity_id).await {
                let bundle_ids = ord.get::<Vec<BundleId>>();
                let bundle_resolver = Arc::clone(&self.bundle_repo);
                let bundles = stream::iter(bundle_ids.iter())
                    .scan((), move |_, bundle_id| {
                        let bundle_resolver = Arc::clone(&bundle_resolver);
                        async move { bundle_resolver.lock().get(*bundle_id).await }
                    })
                    .collect::<Vec<_>>()
                    .await;
                let ctx = self.make_context(pool.box_id()).await;
                let run_result = match (ord.clone(), bundles.first().cloned()) {
                    (Order::Deposit(deposit), _) => deposit
                        .try_run(pool.clone(), (), ctx)
                        .map(|(tx, next_pool, bundle)| (tx, next_pool, vec![bundle], None))
                        .map_err(|err| err.map(Order::Deposit)),
                    (Order::Redeem(redeem), Some(bundle)) => redeem
                        .try_run(pool.clone(), bundle, ctx)
                        .map(|(tx, next_pool, _)| (tx, next_pool, Vec::new(), None))
                        .map_err(|err| err.map(Order::Redeem)),
                    (Order::Compound(compound), _) if !bundles.is_empty() => {
                        let funding = self
                            .funding_repo
                            .lock()
                            .select(compound.estimated_min_value())
                            .await;
                        if let Some(funding) = NonEmpty::from_vec(funding) {
                            compound
                                .try_run(pool.clone(), (bundles.clone(), funding), ctx)
                                .map(|(tx, next_pool, (next_bundles, residual_funding))| {
                                    (tx, next_pool, next_bundles, residual_funding)
                                })
                                .map_err(|err| err.map(Order::Compound))
                        } else {
                            error!("No funding can be found for managed compounding");
                            return;
                        }
                    }
                    (Order::Compound(_), _) => Err(RunOrderError::Fatal(
                        format!("No bundles found for Compound"),
                        ord.clone(),
                    )),
                    (Order::Redeem(redeem), None) => {
                        error!("Bunle not found for Redeem [{:?}]", redeem.get_self_ref());
                        return;
                    }
                };
                match run_result {
                    Ok((tx, next_pool, next_bundles, residual_funding)) => {
                        match self.prover.sign(tx) {
                            Ok(tx) => {
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
                                    self.backlog.lock().recharge(ord).await; // Return order to backlog
                                } else {
                                    pool_resolver
                                        .put(Traced {
                                            state: next_pool,
                                            prev_state_id: Some(pool.get_self_state_ref()),
                                        })
                                        .await;
                                    if let Some(residual_funding) = residual_funding {
                                        self.funding_repo.lock().put_predicted(residual_funding).await;
                                    }
                                    for bundle_st in next_bundles.into_iter().zip_longest(bundles) {
                                        match bundle_st {
                                            EitherOrBoth::Both(next_bundle, prev_bundle) => {
                                                self.bundle_repo
                                                    .lock()
                                                    .put_predicted(Traced {
                                                        state: next_bundle,
                                                        prev_state_id: Some(
                                                            prev_bundle.1.get_self_state_ref(),
                                                        ),
                                                    })
                                                    .await
                                            }
                                            EitherOrBoth::Left(next_bundle) => {
                                                self.bundle_repo
                                                    .lock()
                                                    .put_predicted(Traced {
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
                            Err(prove_err) => {
                                error!("Failed to sign transaction due to {}", prove_err);
                            }
                        }
                    }
                    Err(RunOrderError::NonFatal(err, ord)) => {
                        warn!("Order suspended due to non-fatal error {}", err);
                        self.backlog.lock().suspend(ord).await;
                    }
                    Err(RunOrderError::Fatal(err, ord)) => {
                        warn!("Order dropped due to fatal error {}", err);
                        self.backlog.lock().remove(ord.get_self_ref()).await;
                    }
                }
            }
        }
    }
}
