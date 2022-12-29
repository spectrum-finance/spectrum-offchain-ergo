use std::cell::Cell;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use futures::{stream, StreamExt};
use itertools::{EitherOrBoth, Itertools};
use log::{error, info, trace, warn};
use tokio::sync::Mutex;

use spectrum_offchain::backlog::Backlog;
use spectrum_offchain::box_resolver::persistence::EntityRepo;
use spectrum_offchain::box_resolver::resolve_entity_state;
use spectrum_offchain::data::unique_entity::{Predicted, Traced};
use spectrum_offchain::data::{Has, OnChainEntity, OnChainOrder};
use spectrum_offchain::executor::Executor;
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::network::ErgoNetwork;
use spectrum_offchain::transaction::TransactionCandidate;

use crate::bundle::{resolve_bundle_state, BundleRepo};
use crate::data::bundle::IndexedBundle;
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

pub struct OrderExecutor<TNetwork, TBacklog, TPools, TBundles, TFunding, TProver> {
    network: TNetwork,
    backlog: Arc<Mutex<TBacklog>>,
    pool_repo: Arc<Mutex<TPools>>,
    bundle_repo: Arc<Mutex<TBundles>>,
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
    pub fn new(
        network: TNetwork,
        backlog: Arc<Mutex<TBacklog>>,
        pool_repo: Arc<Mutex<TPools>>,
        bundle_repo: Arc<Mutex<TBundles>>,
        funding_repo: Arc<Mutex<TFunding>>,
        prover: TProver,
        executor_prop: ErgoTree,
    ) -> Self {
        Self {
            network,
            backlog,
            pool_repo,
            bundle_repo,
            funding_repo,
            prover,
            executor_prop,
            context_cache: Cell::new((0, 0)),
        }
    }

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
    TPools: EntityRepo<AsBox<Pool>>,
    TBundles: BundleRepo,
    TFunding: FundingRepo,
    TProver: SigmaProver,
{
    async fn try_execute_next(&mut self) -> Result<(), ()> {
        let next_ord = {
            let mut backlog = self.backlog.lock().await;
            backlog.try_pop().await
        };
        if let Some(ord) = next_ord {
            trace!(target: "offchain_lm", "Order acquired [{:?}]", ord.get_self_ref());
            let entity_id = ord.get_entity_ref();
            if let Some(pool) = resolve_entity_state(entity_id, Arc::clone(&self.pool_repo)).await {
                trace!(target: "offchain_lm", "Pool for order [{:?}] is [{:?}]", ord.get_self_ref(), pool.get_self_ref());
                let conf = pool.1.conf;
                let bundle_ids = ord.get::<Vec<BundleId>>();
                let bundle_resolver = Arc::clone(&self.bundle_repo);
                let bundles = stream::iter(bundle_ids.iter())
                    .scan((), move |_, bundle_id| {
                        let bundle_repo = Arc::clone(&bundle_resolver);
                        async move { resolve_bundle_state(*bundle_id, bundle_repo).await }
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
                            .await
                            .collect(compound.estimated_min_value())
                            .await;
                        if let Ok(funding) = funding {
                            compound
                                .try_run(pool.clone(), (bundles.clone(), funding), ctx)
                                .map(|(tx, next_pool, (next_bundles, residual_funding))| {
                                    (tx, next_pool, next_bundles, residual_funding)
                                })
                                .map_err(|err| err.map(Order::Compound))
                        } else {
                            error!("No funding can be found for managed compounding");
                            return Err(());
                        }
                    }
                    (Order::Compound(_), _) => Err(RunOrderError::Fatal(
                        format!("No bundles found for Compound"),
                        ord.clone(),
                    )),
                    (Order::Redeem(redeem), None) => {
                        error!("Bunle not found for Redeem [{:?}]", redeem.get_self_ref());
                        return Err(());
                    }
                };
                match run_result {
                    Ok((tx, next_pool, next_bundles, residual_funding)) => {
                        trace!(target: "offchain_lm", "Order [{:?}] successfully evaluated", ord.get_self_ref());
                        match self.prover.sign(tx) {
                            Ok(tx) => {
                                if let Err(client_err) = self.network.submit_tx(tx).await {
                                    // Note, here `submit_tx(tx)` can fail not only bc pool state is consumed,
                                    // but also bc bundle is consumed, what is less possible though. That's why
                                    // we just invalidate pool.
                                    // todo: In the future more precise error handling may be possible if we
                                    // todo: implement a way to find out which input failed exactly.
                                    warn!("Execution failed while submitting tx due to {}", client_err);
                                    self.pool_repo
                                        .lock()
                                        .await
                                        .invalidate(pool.get_self_state_ref())
                                        .await;
                                    self.backlog.lock().await.recharge(ord).await;
                                // Return order to backlog
                                } else {
                                    self.pool_repo
                                        .lock()
                                        .await
                                        .put_predicted(Traced {
                                            state: next_pool,
                                            prev_state_id: Some(pool.get_self_state_ref()),
                                        })
                                        .await;
                                    if let Some(residual_funding) = residual_funding {
                                        self.funding_repo
                                            .lock()
                                            .await
                                            .put_predicted(residual_funding)
                                            .await;
                                    }
                                    for bundle_st in next_bundles.into_iter().zip_longest(bundles) {
                                        match bundle_st {
                                            EitherOrBoth::Both(next_bundle, prev_bundle) => {
                                                self.bundle_repo
                                                    .lock()
                                                    .await
                                                    .put_predicted(Traced {
                                                        state: next_bundle.map(|as_box| {
                                                            as_box.map(|b| IndexedBundle::new(b, conf))
                                                        }),
                                                        prev_state_id: Some(
                                                            prev_bundle.1.get_self_state_ref(),
                                                        ),
                                                    })
                                                    .await
                                            }
                                            EitherOrBoth::Left(next_bundle) => {
                                                self.bundle_repo
                                                    .lock()
                                                    .await
                                                    .put_predicted(Traced {
                                                        state: next_bundle.map(|as_box| {
                                                            as_box.map(|b| IndexedBundle::new(b, conf))
                                                        }),
                                                        prev_state_id: None,
                                                    })
                                                    .await
                                            }
                                            EitherOrBoth::Right(_) => {}
                                        }
                                    }
                                }
                            }
                            Err(signing_err) => {
                                error!("Failed to sign transaction due to {}", signing_err);
                            }
                        }
                    }
                    Err(RunOrderError::NonFatal(err, ord)) => {
                        warn!(
                            "Order [{:?}] suspended due to non-fatal error {}",
                            ord.get_self_ref(),
                            err
                        );
                        self.backlog.lock().await.suspend(ord).await;
                    }
                    Err(RunOrderError::Fatal(err, ord)) => {
                        warn!(
                            "Order [{:?}] dropped due to fatal error {}",
                            ord.get_self_ref(),
                            err
                        );
                        self.backlog.lock().await.remove(ord.get_self_ref()).await;
                    }
                }
                return Ok(());
            } else {
                warn!(target: "offchain_lm", "No pool is found for order [{:?}]", ord.get_self_ref());
            }
        }
        Err(())
    }
}
