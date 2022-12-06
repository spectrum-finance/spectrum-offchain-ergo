use std::sync::{Arc, Mutex};

use futures::stream::StreamExt;
use futures::Stream;

use crate::event_sink::types::{DefaultEventHandler, EventHandler};

pub mod handlers;
pub mod types;

pub async fn process_events<'a, TUpstream, TEvent, TDefHan>(
    upstream: TUpstream,
    handlers: Vec<Box<dyn EventHandler<TEvent>>>,
    default_han: TDefHan,
) where
    TUpstream: Stream<Item = TEvent> + 'a,
    TEvent: Clone + 'a,
    TDefHan: DefaultEventHandler<TEvent> + 'a,
{
    let handlers_arc = Arc::new(Mutex::new(handlers));
    let def_handler_arc = Arc::new(Mutex::new(default_han));
    upstream
        .for_each(move |ev| {
            let hans = handlers_arc.clone();
            let def_han = def_handler_arc.clone();
            async move {
                let mut unhandled_ev = None;
                let mut hans_guard = hans.lock().unwrap();
                for han in hans_guard.iter_mut() {
                    let maybe_unhandled_ev = han.try_handle(ev.clone()).await;
                    if unhandled_ev.is_none() {
                        unhandled_ev = maybe_unhandled_ev;
                    }
                }
                if let Some(unhandled_ev) = unhandled_ev {
                    let mut def_han_guard = def_han.lock().unwrap();
                    def_han_guard.handle(unhandled_ev).await;
                }
            }
        })
        .await;
}
