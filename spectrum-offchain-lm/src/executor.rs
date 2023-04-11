use std::cell::Cell;
use std::sync::Arc;

use crate::data::bundle::IndexedBundle;
use crate::data::FundingId;
use crate::token_details::get_token_details;
use async_trait::async_trait;
use chrono::Utc;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use futures::{stream, StreamExt};
use itertools::{EitherOrBoth, Itertools};
use log::{error, info, trace, warn};
use spectrum_offchain::data::order::ProgressingOrder;
use tokio::sync::Mutex;

use spectrum_offchain::backlog::Backlog;
use spectrum_offchain::box_resolver::persistence::EntityRepo;
use spectrum_offchain::box_resolver::resolve_entity_state;
use spectrum_offchain::data::unique_entity::{Predicted, Traced};
use spectrum_offchain::data::{Has, OnChainEntity, OnChainOrder};
use spectrum_offchain::executor::{
    generate_invalidations, parse_err, Executor, Invalidation, NodeSubmitTxError, OrderType, RunOrderError,
};
use spectrum_offchain::network::ErgoNetwork;
use spectrum_offchain::transaction::TransactionCandidate;

use crate::bundle::{resolve_bundle_state, BundleRepo};
use crate::data::context::ExecutionContext;
use crate::data::order::Order;
use crate::data::pool::Pool;
use crate::data::{AsBox, BundleId, BundleStateId};
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

pub trait RunOrder: ConsumeExtra + ProduceExtra + Sized {
    #[allow(clippy::type_complexity)]
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

pub struct OrderExecutor<'a, TNetwork, TBacklog, TPools, TBundles, TFunding, TProver> {
    network: &'a TNetwork,
    backlog: Arc<Mutex<TBacklog>>,
    pool_repo: Arc<Mutex<TPools>>,
    bundle_repo: Arc<Mutex<TBundles>>,
    funding_repo: Arc<Mutex<TFunding>>,
    prover: TProver,
    executor_prop: ErgoTree,
    context_cache: Cell<(u32, i64)>,
}

const CTX_TTL_SECS: i64 = 30;

impl<'a, TNetwork, TBacklog, TPools, TBundles, TFunding, TProver>
    OrderExecutor<'a, TNetwork, TBacklog, TPools, TBundles, TFunding, TProver>
where
    TNetwork: ErgoNetwork,
{
    pub fn new(
        network: &'a TNetwork,
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
impl<'a, TNetwork, TBacklog, TPools, TBundles, TFunding, TProver> Executor
    for OrderExecutor<'a, TNetwork, TBacklog, TPools, TBundles, TFunding, TProver>
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
                info!(
                    target: "offchain_lm",
                    "Pool for order [{:?}] is [{:?}], pool_state: {}",
                    ord.get_self_ref(),
                    pool.get_self_ref(),
                    pool
                );
                info!(
                    "Pool for order [{:?}] is [{:?}], pool_state: {}",
                    ord.get_self_ref(),
                    pool.get_self_ref(),
                    pool
                );
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
                info!("Running against {} with {}", pool, ctx);
                info!(target: "offchain_lm", "Running against {} with {}", pool, ctx);
                let run_result = match (ord.clone(), bundles.first().cloned()) {
                    (Order::Deposit(deposit), _) => {
                        // Try to get token names from node
                        let token_details = get_token_details(
                            pool.1.pool_id,
                            pool.1.budget_rem.token_id,
                            deposit.1.lq.token_id,
                            self.network,
                        )
                        .await;
                        deposit
                            .try_run(pool.clone(), token_details, ctx)
                            .map(|(tx, next_pool, bundle)| {
                                (tx, next_pool, vec![bundle], None, OrderType::Deposit)
                            })
                            .map_err(|err| err.map(Order::Deposit))
                    }
                    (Order::Redeem(redeem), Some(bundle)) => redeem
                        .try_run(pool.clone(), bundle, ctx)
                        .map(|(tx, next_pool, _)| (tx, next_pool, Vec::new(), None, OrderType::Redeem))
                        .map_err(|err| err.map(Order::Redeem)),
                    (Order::Compound(compound), _) if !bundles.is_empty() => {
                        if let Some(pool_epoch_ix) = pool.1.epoch_ix {
                            // The following condition can be true if the bot is delayed in compounding
                            // an epoch 'n' into the time period of the subsequent epoch. We must exit
                            // early and try again once pool box is updated.
                            if pool_epoch_ix < compound.epoch_ix {
                                // Suspend order until newer pool box is confirmed on-chain
                                error!(
                                    "Trying to compound order @ epoch {} with pool box @ epoch {}. Try again later.",
                                    compound.epoch_ix,
                                    pool_epoch_ix
                                );
                                error!(
                                    target: "offchain_lm",
                                    "Trying to compound order @ epoch {} with pool box @ epoch {}. Try again later.",
                                    compound.epoch_ix,
                                    pool_epoch_ix
                                );
                                self.backlog.lock().await.suspend(ord).await;
                                return Err(());
                            }
                        }

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
                                    (tx, next_pool, next_bundles, residual_funding, OrderType::Compound)
                                })
                                .map_err(|err| err.map(Order::Compound))
                        } else {
                            error!("No funding can be found for managed compounding");
                            return Err(());
                        }
                    }
                    (Order::Compound(_), _) => Err(RunOrderError::Fatal(
                        "No bundles found for Compound".to_string(),
                        ord.clone(),
                    )),
                    (Order::Redeem(redeem), None) => {
                        error!("Bundle not found for Redeem [{:?}]", redeem.get_self_ref(),);
                        return Err(());
                    }
                };
                match run_result {
                    Ok((tx, next_pool, next_bundles, residual_funding, order_type)) => {
                        trace!(target: "offchain_lm", "Order [{}] successfully evaluated", ord.get_self_ref());
                        match self.prover.sign(tx) {
                            Ok(tx) => {
                                info!(
                                    target: "offchain_lm", "Transaction ID for {:?} order [{}] is [{}]",
                                    order_type,
                                    ord.get_self_ref(),
                                    tx.id()
                                );
                                info!(
                                    "Transaction ID for {:?} order [{}] is [{}]",
                                    order_type,
                                    ord.get_self_ref(),
                                    tx.id()
                                );
                                for (i, o) in tx.outputs.iter().enumerate() {
                                    trace!(target: "offchain_lm", "tx_output {}: {:?}", i, o.box_id());
                                }
                                if let Err(client_err) = self.network.submit_tx(tx.clone()).await {
                                    warn!("Execution failed while submitting tx due to {}", client_err);
                                    warn!(
                                        target: "offchain_lm",
                                        "Execution failed while submitting tx due to {}",
                                        client_err
                                    );
                                    match parse_err(&client_err.0) {
                                        NodeSubmitTxError::MissingInputs(missing_indices) => {
                                            let invalidations =
                                                generate_invalidations(order_type, missing_indices);

                                            for i in invalidations {
                                                match i {
                                                    Invalidation::Pool => {
                                                        // We suspend the order and also invalidate
                                                        // the pool. If the pool is actually
                                                        // invalid, it's gone.
                                                        //
                                                        // Otherwise we are just waiting for
                                                        // subsequent pool box to be confimed by
                                                        // the ledger. The program will be brought
                                                        // back by `ConfirmedProgramUpdateHandler`.
                                                        self.pool_repo
                                                            .lock()
                                                            .await
                                                            .invalidate(
                                                                pool.get_self_state_ref(),
                                                                pool.get_self_ref(),
                                                            )
                                                            .await;
                                                        self.backlog.lock().await.suspend(ord.clone()).await;
                                                    }

                                                    Invalidation::Funding => {
                                                        assert_eq!(order_type, OrderType::Compound);
                                                        self.funding_repo
                                                            .lock()
                                                            .await
                                                            .remove(FundingId::from(
                                                                tx.inputs.get(1).unwrap().box_id,
                                                            ))
                                                            .await;
                                                    }

                                                    Invalidation::Order => {
                                                        self.backlog
                                                            .lock()
                                                            .await
                                                            .remove(ord.get_self_ref())
                                                            .await;
                                                    }

                                                    Invalidation::StakingBundles(bundles_ix) => {
                                                        for ix in bundles_ix {
                                                            self.bundle_repo
                                                                .lock()
                                                                .await
                                                                .invalidate(BundleStateId::from(
                                                                    tx.inputs.get(ix).unwrap().box_id,
                                                                ))
                                                                .await;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        NodeSubmitTxError::DoubleSpend => {
                                            self.backlog.lock().await.suspend(ord).await;
                                        }
                                        NodeSubmitTxError::Unhandled => (),
                                    }
                                } else {
                                    // Return order to backlog to check for settlement.
                                    {
                                        self.backlog
                                            .lock()
                                            .await
                                            .check_later(ProgressingOrder {
                                                order: ord,
                                                timestamp: Utc::now().timestamp(),
                                            })
                                            .await;
                                    }
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
                                error!(
                                    target: "offchain_lm",
                                    "Failed to sign transaction due to {}",
                                    signing_err
                                );
                            }
                        }
                    }
                    Err(RunOrderError::NonFatal(err, ord)) => {
                        warn!(
                            "Order [{:?}] suspended due to non-fatal error {}",
                            ord.get_self_ref(),
                            err
                        );
                        warn!(
                            target: "offchain_lm",
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
                        warn!(
                            target: "offchain_lm",
                            "Order [{:?}] dropped due to fatal error {}",
                            ord.get_self_ref(),
                            err
                        );
                        self.backlog.lock().await.remove(ord.get_self_ref()).await;
                    }
                }
                return Ok(());
            } else {
                warn!("No pool is found for order [{:?}]", ord.get_self_ref());
                warn!(target: "offchain_lm", "No pool is found for order [{:?}]", ord.get_self_ref());
                self.backlog.lock().await.remove(ord.get_self_ref()).await;
            }
        }
        Err(())
    }
}
