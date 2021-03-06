use super::message_publisher::*;

use futures::prelude::*;
use futures::future::{BoxFuture};
use futures::task::{Poll, Context};

use std::pin::*;

///
/// The `StreamPublisher` struct sends a stream to a publisher. It implements the `Future` trait, and
/// will return an empty value when the entire stream has been sent to the publisher.
///
pub struct StreamPublisher<'a, Publisher, SourceStream> {
    /// The publisher where the stream will be sent to
    publisher: &'a mut Publisher,

    /// Future that's polled when the publisher is closed
    when_closed: BoxFuture<'static, ()>,

    /// The stream that is being sent to the publisher
    stream: Pin<Box<SourceStream>>,

    /// The value that's currently being published to the publisher
    currently_publishing: Option<BoxFuture<'static, ()>>
}

impl<'a, Publisher, SourceStream> StreamPublisher<'a, Publisher, SourceStream>
where   Publisher:      MessagePublisher,
        SourceStream:   'a+Stream<Item=Publisher::Message> {
    ///
    /// Creates a new stream publisher
    ///
    pub fn new(publisher: &'a mut Publisher, stream: SourceStream) -> StreamPublisher<'a, Publisher, SourceStream> {
        StreamPublisher {
            when_closed:            publisher.when_closed(),
            publisher:              publisher,
            stream:                 Box::pin(stream),
            currently_publishing:   None
        }
    }
}

impl<'a, Publisher, SourceStream> Future for StreamPublisher<'a, Publisher, SourceStream>
where   Publisher:      MessagePublisher,
        SourceStream:   Stream<Item=Publisher::Message> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context) -> Poll<()> {
        // If we're currently trying to publish a value, then poll that future first
        let currently_publishing = self.currently_publishing.take();

        if let Some(mut currently_publishing) = currently_publishing {
            // Poll the value we're currently publishing
            match currently_publishing.poll_unpin(context) {
                // If still pending, return pending and keep publishing the value
                Poll::Pending   => { self.currently_publishing = Some(currently_publishing); return Poll::Pending; }

                // If ready, carry on and try to read from the stream
                Poll::Ready(()) => { }
            }
        }

        // Poll when_closed (we check the flag later so the result of this future doesn't matter right now: we just need to make sure it wakes us)
        let _when_closed = self.when_closed.poll_unpin(context);

        // Attempt to read a value from the stream
        loop {
            if self.publisher.is_closed() {
                // If the publisher has closed, then immediately complete the future (and stop reading from the stream)
                return Poll::Ready(());
            }

            match self.stream.poll_next_unpin(context) {
                Poll::Pending => {
                    // Stream is waiting to send more data
                    return Poll::Pending;
                }

                Poll::Ready(None) => {
                    // Stream has finished
                    return Poll::Ready(());
                }

                Poll::Ready(Some(next_message)) => {
                    // Start to send a new message
                    let mut currently_publishing = self.publisher.publish(next_message);

                    // Try to complete the send immediately
                    match currently_publishing.poll_unpin(context) {
                        // If still pending, return pending and keep publishing the value
                        Poll::Pending   => { self.currently_publishing = Some(currently_publishing); return Poll::Pending; }

                        // If ready, carry on and try to read from the stream
                        Poll::Ready(()) => { }
                    }
                }
            }
        }
    }
}

///
/// Provides a way to send the values generated by a stream to a publisher
///
pub trait SendStreamToPublisher : Sized+MessagePublisher {
    ///
    /// Sends everything from a particular source stream to this publisher
    ///
    fn send_all<'a, SourceStream>(&'a mut self, stream: SourceStream) -> StreamPublisher<'a, Self, SourceStream>
    where SourceStream: 'a+Stream<Item=Self::Message>;
}

impl<T: Sized+MessagePublisher> SendStreamToPublisher for T {
    fn send_all<'a, SourceStream>(&'a mut self, stream: SourceStream) -> StreamPublisher<'a, Self, SourceStream>
    where SourceStream: 'a+Stream<Item=Self::Message> {
        StreamPublisher::new(self, stream)
    }
}
