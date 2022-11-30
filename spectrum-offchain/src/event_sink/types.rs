use async_trait::async_trait;

#[async_trait(?Send)]
pub trait EventHandler<TEvent> {
    /// Tries to handle the gicen event if applicable.
    /// Returns `Some(TEvent)` back otherwise.
    async fn try_handle(&mut self, ev: TEvent) -> Option<TEvent>;
}

#[async_trait(?Send)]
pub trait DefaultEventHandler<TEvent> {
    async fn handle(&mut self, ev: TEvent);
}
