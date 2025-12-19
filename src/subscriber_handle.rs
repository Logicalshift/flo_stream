use std::sync::atomic::{AtomicUsize, Ordering};

///
/// Unique identifier for a subscriber to a publisher
///
#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub (crate) struct SubscriberHandle(usize);

impl SubscriberHandle {
    ///
    /// Creates a new subscriber handle (which will be unique and different to any other)
    ///
    pub fn new() -> Self {
        static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(0);

        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);

        Self(handle)
    }
}