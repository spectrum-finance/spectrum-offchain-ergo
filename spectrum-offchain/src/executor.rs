use std::fmt::Display;
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
pub enum RunOrderFailure<TOrd: OnChainOrder> {
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
        ctx: TCtx,
    ) -> Result<(Transaction, Predicted<TEntity>), RunOrderFailure<Self>>;
}

#[async_trait(?Send)]
pub trait Executor<TOrd, TEntity> {
    /// Execute next available order.
    /// Drives execution to completion (submit tx or handle error).
    async fn execute_next(&mut self);
}

pub struct OrderExecutor<TNetwork, TBacklog, TResolver, TCtx> {
    network: TNetwork,
    backlog: TBacklog,
    resolver: TResolver,
    ctx: TCtx,
}

#[derive(Debug, Clone)]
pub struct OrderExecCtx {}

#[async_trait(?Send)]
impl<TOrd, TEntity, TNetwork, TBacklog, TResolver> Executor<TOrd, TEntity>
    for OrderExecutor<TNetwork, TBacklog, TResolver, OrderExecCtx>
where
    TOrd: OnChainOrder + RunOrder<TEntity, OrderExecCtx> + Clone + Display,
    TEntity: OnChainEntity + Clone,
    TOrd::TEntityId: IsEqual<TEntity::TEntityId>,
    TNetwork: ErgoNetwork,
    TBacklog: Backlog<TOrd>,
    TResolver: BoxResolver<TEntity>,
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
                    Err(RunOrderFailure::NonFatal(err, ord)) => {
                        warn!("Order suspended due to non-fatal error {}", err);
                        self.backlog.suspend(ord).await;
                    }
                    Err(RunOrderFailure::Fatal(err, ord_id)) => {
                        warn!("Order dropped due to non-fatal error {}", err);
                        self.backlog.remove(ord_id).await;
                    }
                }
            }
        }
    }
}

/// Construct Executor stream that drives sequential order execution.
pub fn executor_stream<'a, TOrd, TEntity, TExecutor: Executor<TOrd, TEntity> + 'a>(
    executor: TExecutor,
) -> impl Stream<Item = ()> + 'a {
    let executor = Arc::new(Mutex::new(executor));
    stream::iter(0..).then(move |_| {
        let executor = executor.clone();
        async move {
            let mut executor_quard = executor.lock();
            executor_quard.execute_next().await
        }
    })
}
