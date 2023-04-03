use std::pin::Pin;
use std::task::{Context, Poll};

use async_channel::{Sender, TrySendError};
use futures::Sink;

pub struct AsSink<T>(pub Sender<T>);

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SinkErr {
    Closed,
    Full,
}

impl<T> Sink<T> for AsSink<T> {
    type Error = SinkErr;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.0.is_full() {
            Poll::Ready(Err(SinkErr::Full))
        } else if self.0.is_closed() {
            Poll::Ready(Err(SinkErr::Closed))
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(self: Pin<&mut Self>, msg: T) -> Result<(), Self::Error> {
        self.0.try_send(msg).map_err(|err| match err {
            TrySendError::Full(_) => SinkErr::Full,
            TrySendError::Closed(_) => SinkErr::Closed,
        })
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.close();
        Poll::Ready(Ok(()))
    }
}
