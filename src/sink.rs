use super::subscriber::*;
use super::message_publisher::*;

use futures::prelude::*;
use futures::task::{Context, Poll};

use std::pin::{Pin};

///
/// An implementation of the Sink trait that can be applied to publishers
///
pub struct PublisherSink<Publisher>
where Publisher: MessagePublisher {
    /// The publisher that is being turned into a sink
    publisher: Publisher
}

impl<Publisher> PublisherSink<Publisher> 
where Publisher: MessagePublisher {
    ///
    /// Provides access to the underlying MessagePublisher for this sink
    ///
    pub fn as_publisher<'a>(&'a mut self) -> &'a mut Publisher {
        &mut self.publisher
    }

    ///
    /// Creates a subscription to this publisher
    /// 
    /// Any future messages sent here will also be sent to this subscriber.
    /// 
    pub fn subscribe(&mut self) -> Subscriber<Publisher::Message> {
        self.publisher.subscribe()
    }
}

impl<Publisher> Sink<Publisher::Message> for PublisherSink<Publisher>
where Publisher: MessagePublisher {
    type Error = ();

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        unimplemented!()
    }

    fn start_send(self: Pin<&mut Self>, item: Publisher::Message) -> Result<(), Self::Error> {
        unimplemented!()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        unimplemented!()
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        unimplemented!()
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
where Publisher: Sized+MessagePublisher {
    fn to_sink(self) -> PublisherSink<Self> {
        PublisherSink {
            publisher: self
        }
    }
}
