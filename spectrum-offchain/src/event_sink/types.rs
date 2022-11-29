pub trait EventHandler<TEvent> {
    fn try_handle(&mut self, e: TEvent) -> Option<TEvent>;
}

pub trait DefaultEventHandler<TEvent> {
    fn handle(&mut self, e: TEvent);
}
