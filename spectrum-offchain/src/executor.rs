use std::fmt::Display;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use ergo_lib::chain::transaction::unsigned::UnsignedTransaction;
use ergo_lib::chain::transaction::Transaction;
use futures::{stream, Stream, StreamExt};
use log::warn;
use parking_lot::Mutex;
use type_equalities::{trivial_eq, IsEqual};

use crate::backlog::Backlog;
use crate::box_resolver::BoxResolver;
use crate::data::unique_entity::{Predicted, Traced};
use crate::data::{OnChainEntity, OnChainOrder};
use crate::network::ErgoNetwork;
use crate::transaction::IntoTx;

/// Indicated the kind of failure on at attempt to execute an order offline.
pub enum RunOrderError<TOrd> {
    /// Discard order in the case of fatal failure.
    Fatal(String, TOrd),
    /// Return order in the case of non-fatal failure.
    NonFatal(String, TOrd),
}

impl<O> RunOrderError<O> {
    pub fn map<F, O2>(self, f: F) -> RunOrderError<O2>
    where
        F: FnOnce(O) -> O2,
    {
        match self {
            RunOrderError::Fatal(rn, o) => RunOrderError::Fatal(rn, f(o)),
            RunOrderError::NonFatal(rn, o) => RunOrderError::NonFatal(rn, f(o)),
        }
    }
}

pub trait RunOrder<TEntity, TCtx>: OnChainOrder + Sized {
    /// Try to run the given `TOrd` against the given `TEntity`.
    /// Returns transaction and the next state of the persistent entity in the case of success.
    /// Returns `RunOrderError<TOrd>` otherwise.
    fn try_run(
        self,
        entity: TEntity,
        ctx: TCtx, // can be used to pass extra deps
    ) -> Result<(UnsignedTransaction, Predicted<TEntity>), RunOrderError<Self>>;
}

#[async_trait(?Send)]
pub trait Executor {
    /// Execute next available order.
    /// Drives execution to completion (submit tx or handle error).
    async fn execute_next(&mut self);
}

/// A generic executor suitable for cases when single order is applied to a signle entity (pool).
pub struct OrderExecutor<TNetwork, TBacklog, TResolver, TCtx, TOrd, TEntity> {
    network: TNetwork,
    backlog: TBacklog,
    resolver: TResolver,
    ctx: TCtx,
    pd1: PhantomData<TOrd>,
    pd2: PhantomData<TEntity>,
}

#[async_trait(?Send)]
impl<'a, TNetwork, TBacklog, TResolver, TCtx, TOrd, TEntity> Executor
    for OrderExecutor<TNetwork, TBacklog, TResolver, TCtx, TOrd, TEntity>
where
    TOrd: OnChainOrder + RunOrder<TEntity, TCtx> + Clone + Display,
    TEntity: OnChainEntity + Clone,
    TOrd::TEntityId: IsEqual<TEntity::TEntityId>,
    TNetwork: ErgoNetwork,
    TBacklog: Backlog<TOrd>,
    TResolver: BoxResolver<TEntity>,
    TCtx: Clone,
{
    async fn execute_next(&mut self) {
        if let Some(ord) = self.backlog.try_pop().await {
            let entity_id = ord.get_entity_ref();
            if let Some(entity) = self.resolver.get(trivial_eq().coerce(entity_id)).await {
                match ord.clone().try_run(entity.clone(), self.ctx.clone()) {
                    Ok((tx, next_entity_state)) => {
                        if let Err(err) = self.network.submit_tx(tx.into_tx_without_proofs()).await {
                            warn!("Execution failed while submitting tx due to {}", err);
                            self.resolver
                                .invalidate(entity.get_self_ref(), entity.get_self_state_ref())
                                .await;
                            self.backlog.recharge(ord).await; // Return order to backlog
                        } else {
                            self.resolver
                                .put(Traced {
                                    state: next_entity_state,
                                    prev_state_id: Some(entity.get_self_state_ref()),
                                })
                                .await;
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

/// Construct Executor stream that drives sequential order execution.
pub fn executor_stream<'a, TExecutor: Executor + 'a>(executor: TExecutor) -> impl Stream<Item = ()> + 'a {
    let executor = Arc::new(Mutex::new(executor));
    stream::iter(0..).then(move |_| {
        let executor = executor.clone();
        async move {
            let mut executor_quard = executor.lock();
            executor_quard.execute_next().await
        }
    })
}
