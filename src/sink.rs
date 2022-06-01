use super::subscriber::*;
use super::message_publisher::*;

use futures::prelude::*;
use futures::task::{Context, Poll};
use futures::future::{BoxFuture};

use std::pin::{Pin};

///
/// An implementation of the Sink trait that can be applied to publishers
///
pub struct PublisherSink<Publisher>
where 
    Publisher: MessagePublisher
{
    /// The publisher that is being turned into a sink
    publisher: Option<Publisher>,

    /// Future for awaiting the message sender
    future_sender: Option<BoxFuture<'static, MessageSender<Publisher::Message>>>,

    /// The sender returned by poll_ready
    next_sender: Option<MessageSender<Publisher::Message>>,

    /// The future waiting for the publisher to flush
    future_flush: Option<BoxFuture<'static, ()>>
}

impl<Publisher> PublisherSink<Publisher> 
where
    Publisher: MessagePublisher
{
    ///
    /// Provides access to the underlying MessagePublisher for this sink
    ///
    pub fn as_publisher<'a>(&'a mut self) -> Option<&'a mut Publisher> {
        self.publisher.as_mut()
    }

    ///
    /// Creates a subscription to this publisher
    /// 
    /// Any future messages sent here will also be sent to this subscriber. Returns None if the sink has been closed
    /// 
    pub fn subscribe(&mut self) -> Option<Subscriber<Publisher::Message>> {
        self.publisher.as_mut().map(|publisher| publisher.subscribe())
    }

    ///
    /// Reserves a space for a message with the subscribers, returning when it's ready
    ///
    pub fn when_ready(&mut self) -> Option<BoxFuture<'static, MessageSender<Publisher::Message>>> {
        self.publisher.as_mut().map(|publisher| publisher.when_ready())
    }

    ///
    /// Waits until all subscribers have consumed all pending messages
    /// 
    /// Returns None if the sink has been closed.
    ///
    pub fn when_empty(&mut self) -> Option<BoxFuture<'static, ()>> {
        self.publisher.as_mut().map(|publisher| publisher.when_empty())
    }

    ///
    /// Publishes a message to the subscribers of this object 
    /// 
    /// Returns None if the sink has been closed
    ///
    pub fn publish(&mut self, message: Publisher::Message) -> Option<BoxFuture<'static, ()>> {
        self.publisher.as_mut().map(|publisher| publisher.publish(message))
    }
}

impl<Publisher> Sink<Publisher::Message> for PublisherSink<Publisher>
where 
    Publisher:  MessagePublisher,
    Self:       Unpin,
{
    type Error = ();

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        // Get or create the future sender (get_or_insert_with won't work here due to the multiple borrow of self)
        let future_sender   = match self.future_sender {
            Some(ref mut future_sender) => future_sender,
            None                        => {
                self.future_sender = self.when_ready();
                if self.future_sender.is_none() { return Poll::Ready(Err(())); }
                self.future_sender.as_mut().unwrap()
            }
        };

        // Poll for the next sender and ready it if possible
        match future_sender.poll_unpin(cx) {
            Poll::Ready(sender) => {
                self.future_sender  = None;
                self.next_sender    = Some(sender);
                Poll::Ready(Ok(()))
            },

            Poll::Pending       => Poll::Pending
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: Publisher::Message) -> Result<(), Self::Error> {
        // Send to the next sender if one has been prepared by calling poll_ready
        self.next_sender.take().map(move |sender| sender.send(item));
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        // Get or create the flush future (get_or_insert_with won't work here due to the multiple borrow of self)
        let future_flush    = match self.future_flush {
            Some(ref mut future_flush)  => future_flush,
            None                        => {
                self.future_flush = self.when_empty();
                if self.future_flush.is_none() { return Poll::Ready(Err(())); }
                self.future_flush.as_mut().unwrap()
            }
        };

        // Poll the future for when the publisher is empty
        match future_flush.poll_unpin(cx) {
            Poll::Ready(_)  => {
                self.future_flush  = None;
                Poll::Ready(Ok(()))
            },

            Poll::Pending   => Poll::Pending
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        // This is the same as poll_flush, but the use of Pin prevents us from calling that and unsetting the future
        // The flush future will keep the publisher around until all the subscriber messages are processed

        // Get or create the flush future (get_or_insert_with won't work here due to the multiple borrow of self)
        let future_flush    = match self.future_flush {
            Some(ref mut future_flush)  => future_flush,
            None                        => {
                self.future_flush = self.when_empty();
                if self.future_flush.is_none() { return Poll::Ready(Err(())); }
                self.future_flush.as_mut().unwrap()
            }
        };

        // Need to poll here as we can't set publisher to none while we've borrowed the future_flush future
        let result = future_flush.poll_unpin(cx);

        // Unset the publisher so it's dropped when the flush is done
        self.publisher      = None;
        self.future_sender  = None;
        self.next_sender    = None;

        // Poll the future for when the publisher is empty
        match result {
            Poll::Ready(_)  => {
                self.future_flush  = None;
                Poll::Ready(Ok(()))
            },

            Poll::Pending   => Poll::Pending
        }
    }
}

///
/// Trait that turns publishers into sinks
///
pub trait ToPublisherSink : Sized+MessagePublisher {
    ///
    /// Converts this publisher into a futures Sink
    ///
    fn to_sink(self) -> PublisherSink<Self>;
}

impl<Publisher> ToPublisherSink for Publisher
where
    Publisher: Sized + MessagePublisher
{
    fn to_sink(self) -> PublisherSink<Self> {
        PublisherSink {
            publisher:      Some(self),
            future_sender:  None,
            next_sender:    None,
            future_flush:   None,
        }
    }
}
