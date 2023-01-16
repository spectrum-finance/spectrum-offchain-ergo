use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::sync::{Arc, Once};
use std::time::Duration;

use async_trait::async_trait;
use futures::{stream, Stream};
use futures_timer::Delay;
use log::{trace, warn};
use tokio::sync::Mutex;
use type_equalities::{trivial_eq, IsEqual};

use crate::backlog::Backlog;
use crate::box_resolver::persistence::EntityRepo;
use crate::box_resolver::resolve_entity_state;
use crate::data::unique_entity::{Predicted, Traced};
use crate::data::{OnChainEntity, OnChainOrder};
use crate::network::ErgoNetwork;
use crate::transaction::{TransactionCandidate, UnsignedTransactionOps};

/// Indicated the kind of failure on at attempt to execute an order offline.
#[derive(Debug, PartialEq, Eq)]
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
    ) -> Result<(TransactionCandidate, Predicted<TEntity>), RunOrderError<Self>>;
}

#[async_trait(?Send)]
pub trait Executor {
    /// Execute next available order.
    /// Drives execution to completion (submit tx or handle error).
    async fn try_execute_next(&mut self) -> Result<(), ()>;
}

/// A generic executor suitable for cases when single order is applied to a signle entity (pool).
pub struct OrderExecutor<TNetwork, TBacklog, TEntities, TCtx, TOrd, TEntity> {
    network: TNetwork,
    backlog: TBacklog,
    entity_repo: Arc<Mutex<TEntities>>,
    ctx: TCtx,
    pd1: PhantomData<TOrd>,
    pd2: PhantomData<TEntity>,
}

#[async_trait(?Send)]
impl<'a, TNetwork, TBacklog, TEntities, TCtx, TOrd, TEntity> Executor
    for OrderExecutor<TNetwork, TBacklog, TEntities, TCtx, TOrd, TEntity>
where
    TOrd: OnChainOrder + RunOrder<TEntity, TCtx> + Clone + Display,
    TEntity: OnChainEntity + Clone,
    TEntity::TEntityId: Copy,
    TOrd::TEntityId: IsEqual<TEntity::TEntityId>,
    TNetwork: ErgoNetwork,
    TBacklog: Backlog<TOrd>,
    TEntities: EntityRepo<TEntity>,
    TCtx: Clone,
{
    async fn try_execute_next(&mut self) -> Result<(), ()> {
        if let Some(ord) = self.backlog.try_pop().await {
            let entity_id = ord.get_entity_ref();
            if let Some(entity) =
                resolve_entity_state(trivial_eq().coerce(entity_id), Arc::clone(&self.entity_repo)).await
            {
                match ord.clone().try_run(entity.clone(), self.ctx.clone()) {
                    Ok((tx, next_entity_state)) => {
                        let mut entity_repo = self.entity_repo.lock().await;
                        if let Err(err) = self.network.submit_tx(tx.into_tx_without_proofs()).await {
                            warn!("Execution failed while submitting tx due to {}", err);
                            entity_repo
                                .invalidate(entity.get_self_state_ref(), entity.get_self_ref())
                                .await;
                            self.backlog.recharge(ord).await; // Return order to backlog
                        } else {
                            entity_repo
                                .put_predicted(Traced {
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
                return Ok(());
            }
        }
        Err(())
    }
}

const THROTTLE_SECS: u64 = 1;

/// Construct Executor stream that drives sequential order execution.
pub fn executor_stream<'a, TExecutor: Executor + 'a>(
    executor: TExecutor,
    tip_reached_signal: &'a Once,
) -> impl Stream<Item = ()> + 'a {
    let executor = Arc::new(Mutex::new(executor));
    stream::unfold((), move |_| {
        let executor = executor.clone();
        async move {
            if tip_reached_signal.is_completed() {
                trace!(target: "offchain_lm", "Trying to execute next order ..");
                let mut executor_quard = executor.lock().await;
                if let Err(_) = executor_quard.try_execute_next().await {
                    trace!(target: "offchain_lm", "Execution attempt failed, throttling ..");
                    Delay::new(Duration::from_secs(THROTTLE_SECS)).await;
                }
            } else {
                Delay::new(Duration::from_secs(THROTTLE_SECS)).await;
            }
            Some(((), ()))
        }
    })
}
