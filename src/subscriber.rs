use super::pubsub_core::*;
use super::subscriber_handle::*;

use futures::*;
use futures::task;
use futures::task::{Poll};

use std::sync::*;
use std::pin::{Pin};
use std::collections::{HashMap};

///
/// Represents a subscriber stream from a publisher sink
/// 
pub struct Subscriber<Message> {
    /// The publisher core (shared between all subscribers)
    /// 
    /// Note that when locking the pub_core must always be locked first (if it needs to be locked)
    pub_core: Weak<Mutex<PubCore<Message>>>,

    /// The subscriber core (used only by this subscriber)
    /// 
    /// Note that when locking the pub_core must always be locked first (if it needs to be locked)
    sub_core: Arc<Mutex<SubCore<Message>>>
}

impl<Message> Subscriber<Message> {
    ///
    /// Creates a new subscriber
    /// 
    pub (crate) fn new(pub_core: Weak<Mutex<PubCore<Message>>>, sub_core: Arc<Mutex<SubCore<Message>>>) -> Subscriber<Message> {
        Subscriber {
            pub_core,
            sub_core
        }
    }
}

impl<Message: Clone> Subscriber<Message> {
    ///
    /// Resubscribes to the same publisher as this stream.
    /// 
    /// The new subscriber will receive any future messages that are also  destined for this stream, but will not
    /// receive any past messages.
    /// 
    pub fn resubscribe(&self) -> Self {
        let pub_core = self.pub_core.upgrade();

        if let Some(pub_core) = pub_core {
            let new_sub_core = {
                // Lock the cores
                let mut pub_core    = pub_core.lock().unwrap();
                let sub_core        = self.sub_core.lock().unwrap();

                // Assign an ID
                let new_id = SubscriberHandle::new();

                // Generate a new core for the clone
                let new_sub_core = SubCore {
                    id:                 new_id,
                    published:          true,
                    waiting:            sub_core.waiting.clone(),
                    reserved:           0,
                    notify_waiting:     HashMap::new(),
                    notify_ready:       HashMap::new(),
                };
                let new_sub_core = Arc::new(Mutex::new(new_sub_core));

                // Store in the publisher core
                pub_core.subscribers.insert(new_id, Arc::clone(&new_sub_core));

                new_sub_core
            };

            // Create the new subscriber
            Subscriber {
                pub_core: Arc::downgrade(&pub_core),
                sub_core: new_sub_core
            }
        } else {
            // Publisher has gone away
            let sub_core = self.sub_core.lock().unwrap();

            // Create the new core (no publisher)
            let new_sub_core = SubCore {
                id:                 SubscriberHandle::new(),
                published:          false,
                waiting:            sub_core.waiting.clone(),
                reserved:           0,
                notify_waiting:     HashMap::new(),
                notify_ready:       HashMap::new(),
            };

            // Generate a new subscriber with this core
            Subscriber {
                pub_core: Weak::new(),
                sub_core: Arc::new(Mutex::new(new_sub_core))
            }
        }
    }
}

impl<Message> Drop for Subscriber<Message> {
    fn drop(&mut self) {
        let notify_ready = {
            // Lock the publisher and subscriber cores (note that the publisher core must always be locked first)
            let pub_core = self.pub_core.upgrade();

            if let Some(pub_core) = pub_core {
                // Lock the cores
                let mut pub_core = pub_core.lock().unwrap();
                let mut sub_core = self.sub_core.lock().unwrap();

                // Remove this subscriber from the publisher core
                pub_core.subscribers.remove(&sub_core.id);

                // Need to notify the core if it's waiting on this subscriber (might now be unblocked)
                let notify_ready    = sub_core.notify_ready.drain().map(|(_, waker)| waker).collect::<Vec<_>>();
                notify_ready
            } else {
                // Need to notify the core if it's waiting on this subscriber (might now be unblocked)
                let mut sub_core = self.sub_core.lock().unwrap();
                let notify_ready    = sub_core.notify_ready.drain().map(|(_, waker)| waker).collect::<Vec<_>>();
                notify_ready
            }
        };

        // After releasing the locks, notify the publisher if it's waiting on this subscriber
        notify_ready.into_iter().for_each(|notify| notify.wake());
    }
}

impl<Message> Stream for Subscriber<Message> {
    type Item   = Message;

    fn poll_next(self: Pin<&mut Self>, context: &mut task::Context) -> Poll<Option<Message>> {
        let (result, notify_ready) = {
            // Try to read a message from the waiting list
            let mut sub_core    = self.sub_core.lock().unwrap();
            let handle          = sub_core.id;
            let next_message    = sub_core.waiting.pop_front();

            if let Some(next_message) = next_message {
                // If something is waiting for this subscriber to become ready, then notify it as well
                let notify_ready = sub_core.notify_ready.drain().map(|(_, waker)| waker).collect::<Vec<_>>();

                // Return the next message if it's available
                (Poll::Ready(Some(next_message)), notify_ready)
            } else if !sub_core.published {
                // Stream has finished if the publisher core is no longer available
                (Poll::Ready(None), vec![])
            } else {
                // If the publisher is still alive and there are no messages available, store notification and carry on
                sub_core.notify_waiting.insert(handle, context.waker().clone());

                // If anything is waiting for this subscriber to become ready, make sure it's notified
                let notify_ready = sub_core.notify_ready.drain().map(|(_, waker)| waker).collect::<Vec<_>>();

                (Poll::Pending, notify_ready)
            }
        };

        // If there's something to notify as a result of this request, do so (note that we do this after releasing the core lock)
        notify_ready.into_iter().for_each(|ready| ready.wake());

        // Return the result
        result
    }
}

///
/// It's possible to clone a subscriber stream. The clone will receive any waiting messages
/// and any future messages for the original subscriber
/// 
impl<Message: Clone> Clone for Subscriber<Message> {
    fn clone(&self) -> Subscriber<Message> {
        // TODO: 'clone' is perhaps not right as we don't reproduce the stream in its entirety. Remove this in future versions.
        // 'resubscribe' is a better description of what is happening here.
        self.resubscribe()
    }
}
