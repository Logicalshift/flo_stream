use super::publisher::*;
use super::subscriber::*;
use super::message_publisher::*;

use futures::*;
use futures::future::{BoxFuture};
use futures::channel::oneshot;

///
/// A blocking publisher is a publisher that blocks messages until it has enough subscribers
/// 
/// This is useful for cases where a publisher is being used asynchronously and wants to ensure that
/// its messages are sent to at least one subscriber. Once the required number of subscribers is
/// reached, this will not block again even if some subscribers are dropped.
/// 
pub struct BlockingPublisher<Message> {
    /// True if there are not currently enouogh subscribers in this publisher
    insufficient_subscribers: bool,

    /// The number of required subscribers
    required_subscribers: usize,

    /// The publisher where messages will be relayed
    publisher: Publisher<Message>,

    /// Futures to be notified when there are enough subscribers for this publisher
    notify_futures: Vec<oneshot::Sender<()>>
}

impl<Message: Clone> BlockingPublisher<Message> {
    ///
    /// Creates a new blocking publisher
    /// 
    /// This publisher will refuse to receive any items until at least required_subscribers are connected.
    /// The buffer size indicates the number of queued items permitted per buffer.
    /// 
    pub fn new(required_subscribers: usize, buffer_size: usize) -> BlockingPublisher<Message> {
        BlockingPublisher {
            insufficient_subscribers:   required_subscribers != 0,
            required_subscribers:       required_subscribers,
            publisher:                  Publisher::new(buffer_size),
            notify_futures:             vec![]
        }
    }

    ///
    /// Returns a future that will complete when this publisher has enough subscribers
    /// 
    /// This is useful as a way to avoid blocking with `wait_send` when setting up the publisher
    /// 
    pub fn when_fully_subscribed(&mut self) -> impl Future<Output=Result<(), oneshot::Canceled>>+Send {
        let receiver =  if self.insufficient_subscribers {
            // Return a future that will be notified when we have enough subscribers
            let (sender, receiver) = oneshot::channel();

            // Notify when there are enough subscribers
            self.notify_futures.push(sender);

            Some(receiver)
        } else {
            None
        };

        async {
            if let Some(receiver) = receiver {
                receiver.await
            } else {
                Ok(())
            }
        }
    }
}

impl<Message: 'static+Send+Clone> MessagePublisher for BlockingPublisher<Message> {
    type Message = Message;

    fn subscribe(&mut self) -> Subscriber<Message> {
        // Create the subscription
        let subscription = self.publisher.subscribe();

        // Wake the sink if we get enough subscribers
        if self.insufficient_subscribers && self.publisher.count_subscribers() >= self.required_subscribers {
            // We now have enough subscribers
            self.insufficient_subscribers = false;
            
            // Notify any futures that are waiting on this publisher
            self.notify_futures.drain(..)
                .for_each(|sender| { sender.send(()).ok(); });
        }

        // Result is our new subscription
        subscription
    }

    ///
    /// Reserves a space for a message with the subscribers, returning when it's ready
    ///
    fn when_ready(&mut self) -> BoxFuture<'static, MessageSender<Message>> {
        // If there are not enough subscribers, wait for there to be enough subscribers
        let when_subscribed = if self.insufficient_subscribers {
            Some(self.when_fully_subscribed())
        } else {
            None
        };
        let when_ready = self.publisher.when_ready();

        // Wait for there to be enough subscribers before waiting for the publisher to become ready
        Box::pin(async move {
            if let Some(when_subscribed) = when_subscribed {
                when_subscribed.await.ok();
            }

            when_ready.await
        })
    }

    ///
    /// Waits until all subscribers have consumed all pending messages
    ///
    fn when_empty(&mut self) -> BoxFuture<'static, ()> {
        let when_subscribed = if self.insufficient_subscribers {
            Some(self.when_fully_subscribed())
        } else {
            None
        };
        let when_empty = self.publisher.when_empty();

        Box::pin(async move {
            if let Some(when_subscribed) = when_subscribed {
                when_subscribed.await.ok();
            }

            when_empty.await
        })
    }

    ///
    /// Returns true if this publisher is closed (will not publish any further messages to its subscribers)
    ///
    fn is_closed(&self) -> bool { self.publisher.is_closed() }

    ///
    /// Future that returns when this publisher is closed
    ///
    fn when_closed(&self) -> BoxFuture<'static, ()> {
        self.publisher.when_closed()
    }
}
