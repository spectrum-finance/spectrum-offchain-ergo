pub trait EventHandler<TEvent> {
    /// Tries to handle the given event if applicable.
    /// Returns `Some(TEvent)` back otherwise.
    fn try_handle(&mut self, ev: TEvent) -> Option<TEvent>;
}

pub trait DefaultEventHandler<TEvent> {
    fn handle(&mut self, ev: TEvent);
}
