use futures::stream::StreamExt;
use futures::Stream;

use crate::event_sink::types::{DefaultEventHandler, EventHandler};

pub mod types;
pub mod handlers;

pub async fn process_events<'a, TUpstream, TEvent, TDefHan>(
    upstream: TUpstream,
    mut handlers: Vec<Box<dyn EventHandler<TEvent>>>,
    mut default_han: TDefHan,
) where
    TUpstream: Stream<Item = TEvent> + 'a,
    TEvent: Clone + 'a,
    TDefHan: DefaultEventHandler<TEvent> + 'a,
{
    upstream
        .for_each(move |ev| {
            let mut unhandled_ev = None;
            for han in &mut handlers {
                let maybe_unhandled_ev = han.try_handle(ev.clone());
                if unhandled_ev.is_none() {
                    unhandled_ev = maybe_unhandled_ev;
                }
            }
            if let Some(unhandled_ev) = unhandled_ev {
                default_han.handle(unhandled_ev);
            }
            futures::future::ready(())
        })
        .await;
}
