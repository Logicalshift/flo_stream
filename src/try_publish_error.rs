///
/// The errors that can occur when trying to publish a message
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub (crate) enum PublishError {
    /// The subscribers can accept no more messages
    Full,

    /// The publisher has no subscribers to publish to yet
    NoSubscribers,

    /// The publisher has been closed
    Closed,
}

///
/// The error type returned from `try_publish`
///
pub struct TryPublishError<Message> {
    inner: Message,
    error: PublishError,
}

impl<Message> TryPublishError<Message> {
    ///
    /// Creates a new TryPublishError for the specified failure condition
    ///
    pub (crate) fn with_error(message: Message, error: PublishError) -> TryPublishError<Message> {
        TryPublishError {
            inner: message,
            error: error
        }
    }

    ///
    /// True if the error resulted from the channel being full
    ///
    /// The message can still be sent with blocking by calling `publish()` or `when_ready()`
    ///
    pub fn is_full(&self) -> bool { self.error == PublishError::Full }

    ///
    /// True if this error ocurred because there were no subscribers
    ///
    pub fn no_subscribers(&self) -> bool { self.error == PublishError::NoSubscribers }

    ///
    /// True if this error occurred because the publisher has been closed
    ///
    pub fn is_closed(&self) -> bool { self.error == PublishError::Closed }

    ///
    /// Returns the message that failed to send
    ///
    pub fn into_inner(self) -> Message { 
        self.inner
    }
}
