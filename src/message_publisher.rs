use super::subscriber::*;

use futures::future::{BoxFuture};

///
/// A message sender represents a reserved space for sending a message. Because the
/// space is reserved, the message can be sent immediately
///
pub struct MessageSender<Message> {
    /// Callback that sends the message
    send_message: Option<Box<dyn FnOnce(Message) -> ()+Send>>,

    /// Callback that abandons sending the message
    cancel_send: Option<Box<dyn FnOnce() -> ()+Send>>,

    /// Set to true once the message has been sent
    sent: bool
}

///
/// Trait that provides functions for publishing messages to subscribers
/// 
pub trait MessagePublisher
where   Self:       Send {
    type Message: 'static+Send;

    ///
    /// Creates a subscription to this publisher
    /// 
    /// Any future messages sent here will also be sent to this subscriber.
    /// 
    fn subscribe(&mut self) -> Subscriber<Self::Message>;

    ///
    /// Reserves a space for a message with the subscribers, returning when it's ready
    ///
    fn when_ready(&mut self) -> BoxFuture<'static, MessageSender<Self::Message>>;

    ///
    /// Waits until all subscribers have consumed all pending messages
    ///
    fn when_empty(&mut self) -> BoxFuture<'static, ()>;

    ///
    /// Returns true if this publisher is closed (will not publish any further messages to its subscribers)
    ///
    fn is_closed(&self) -> bool;

    ///
    /// Future that returns when this publisher is closed
    ///
    fn when_closed(&self) -> BoxFuture<'static, ()>;

    ///
    /// Publishes a message to the subscribers of this object 
    ///
    fn publish(&mut self, message: Self::Message) -> BoxFuture<'static, ()> {
        let when_ready = self.when_ready();

        Box::pin(async move {
            let sender = when_ready.await;
            sender.send(message);
        })
    }
}

impl<Message> MessageSender<Message> {
    ///
    /// Creates a new message sender that will perform the supplied actions when the message is sent
    ///
    pub fn new<TSendMsg, TCancelSend>(send_msg: TSendMsg, cancel_send: TCancelSend) -> MessageSender<Message>
    where   TSendMsg:       'static+Send+FnOnce(Message) -> (),
            TCancelSend:    'static+Send+FnOnce() -> () {
        MessageSender {
            send_message:   Some(Box::new(send_msg)),
            cancel_send:    Some(Box::new(cancel_send)),
            sent:           false
        }
    }

    /// 
    /// Sends a message, consuming this object
    /// 
    #[inline]
    pub fn send(mut self, message: Message) {
        self.sent = true;
        self.send_message.take().map(move |send_message| send_message(message));
    }
}

impl<Message> Drop for MessageSender<Message> {
    fn drop(&mut self) {
        if !self.sent {
            self.cancel_send.take().map(move |cancel_send| cancel_send());
        }
    }
}
