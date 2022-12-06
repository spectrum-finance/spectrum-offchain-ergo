use std::fmt::Display;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
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

/// Indicated the kind of failure on at attempt to execute an order offline.
pub enum RunOrderError<TOrd: OnChainOrder> {
    /// Discard order in the case of fatal failure.
    Fatal(String, TOrd::TOrderId),
    /// Return order in the case of non-fatal failure.
    NonFatal(String, TOrd),
}

pub trait RunOrder<TEntity, TCtx>: OnChainOrder + Sized {
    /// Try to run the given `TOrd` against the given `TEntity`.
    /// Returns transaction and the next state of the persistent entity in the case of success.
    /// Returns `RunOrderError<TOrd>` otherwise.
    fn try_run(
        self,
        entity: TEntity,
        ctx: TCtx, // can be used to pass extra deps
    ) -> Result<(Transaction, Predicted<TEntity>), RunOrderError<Self>>;
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
impl<TNetwork, TBacklog, TResolver, TCtx, TOrd, TEntity> Executor
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
                        if let Err(err) = self.network.submit_tx(tx).await {
                            warn!("Execution failed while submitting tx due to {}", err);
                            self.resolver
                                .invalidate(entity.get_self_ref(), entity.get_self_state_ref())
                                .await;
                            self.backlog.recharge(ord).await; // Return order to backlog
                        } else {
                            self.resolver
                                .put(Traced {
                                    state: next_entity_state,
                                    prev_state_id: entity.get_self_state_ref(),
                                })
                                .await;
                        }
                    }
                    Err(RunOrderError::NonFatal(err, ord)) => {
                        warn!("Order suspended due to non-fatal error {}", err);
                        self.backlog.suspend(ord).await;
                    }
                    Err(RunOrderError::Fatal(err, ord_id)) => {
                        warn!("Order dropped due to fatal error {}", err);
                        self.backlog.remove(ord_id).await;
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
