use super::subscriber::*;

use futures::*;
use futures::future::{BoxFuture};

///
/// A message sender represents a reserved space for sending a message. Because the
/// space is reserved, the message can be sent immediately
///
pub struct MessageSender<Message> {
    /// Callback that sends the message
    send_message: Box<dyn FnOnce(Message) -> ()>,

    /// Callback that abandons sending the message
    cancel_send: Box<dyn FnOnce() -> ()>,

    /// Set to true once the message has been sent
    sent: bool
}

///
/// Trait that provides functions for publishing messages to subscribers
/// 
pub trait PublisherSink<Message>
where   Message:    'static+Send,
        Self:       Send {
    ///
    /// Creates a subscription to this publisher
    /// 
    /// Any future messages sent here will also be sent to this subscriber.
    /// 
    fn subscribe(&mut self) -> Subscriber<Message>;

    ///
    /// Reserves a space for a message with the subscribers, returning when it's ready
    ///
    fn when_ready(&mut self) -> BoxFuture<'static, MessageSender<Message>>;

    ///
    /// Waits until all subscribers have consumed all pending messages
    ///
    fn when_empty(&mut self) -> BoxFuture<'static, ()>;

    ///
    /// Publishes a message to the subscribers of this object 
    ///
    fn publish(&mut self, message: Message) -> BoxFuture<'static, ()> {
        let when_ready = self.when_ready();

        Box::pin(async move {
            let sender = when_ready.await;
            sender.send(message);
        })
    }
}

impl<Message> MessageSender<Message> {
    /// 
    /// Sends a message, consuming this object
    /// 
    #[inline]
    pub fn send(self, message: Message) {
        self.sent = true;
        (self.send_message)(message)
    }
}
