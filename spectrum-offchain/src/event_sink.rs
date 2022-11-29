use futures::stream::StreamExt;
use futures::Stream;

use crate::event_sink::types::{DefaultEventHandler, EventHandler};

pub mod types;

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
        .for_each(move |e| {
            for h in &mut handlers {
                if let Some(e) = h.try_handle(e.clone()) {
                    default_han.handle(e)
                }
            }
            futures::future::ready(())
        })
        .await;
}
