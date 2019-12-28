use super::subscriber::*;
use super::pubsub_core::*;
use super::publisher_sink::*;

use futures::*;

use std::sync::*;
use std::collections::{HashMap, VecDeque};

///
/// An 'expiring' publisher is one that responds to backpressure from its subscribers by
/// expiring the most recent message.
/// 
/// Usually when a subscriber stalls in processing, a publisher will refuse to accept
/// further messages and block. This will avoid blocking by instead expiring messages
/// that cannot be processed.
/// 
/// This is useful in a few situations. One important example is distributing state: say
/// you want to indicate to another thread what your current state is, but if it's busy 
/// you don't want to wait for it to consume the previous state before you can finish 
/// updating the latest state.
/// 
/// Another example is signalling. An ExpiringPublisher<()> can be used to signal that an
/// event has occurred but will not block if all subscribers have not responded in the
/// case where the event occurs multiple times.
///
pub struct ExpiringPublisher<Message> {
    /// The shared core of this publisher
    core: Arc<Mutex<PubCore<Message>>>
}

impl<Message: Clone> ExpiringPublisher<Message> {
    ///
    /// Creates a new expiring publisher with a particular buffer size
    /// 
    /// Once a subscriber has buffer_size messages, this publisher will start to drop the
    /// oldest messages.
    /// 
    pub fn new(buffer_size: usize) -> ExpiringPublisher<Message> {
        // Create the core
        let core = PubCore {
            next_subscriber_id: 0,
            publisher_count:    1,
            subscribers:        HashMap::new(),
            max_queue_size:     buffer_size
        };

        // Build the publisher itself
        ExpiringPublisher {
            core:   Arc::new(Mutex::new(core))
        }
    }

    ///
    /// Counts the number of subscribers in this publisher
    /// 
    pub fn count_subscribers(&self) -> usize {
        self.core.lock().unwrap().subscribers.len()
    }

    ///
    /// Creates a duplicate publisher that can be used to publish to the same streams as this object
    /// 
    pub fn republish(&self) -> Self {
        self.core.lock().unwrap().publisher_count += 1;

        ExpiringPublisher {
            core:   Arc::clone(&self.core)
        }
    }
}

impl<Message: 'static+Send+Clone> PublisherSink<Message> for ExpiringPublisher<Message> {
    ///
    /// Subscribes to this publisher
    /// 
    /// Subscribers only receive messages sent to the publisher after they are created.
    /// 
    fn subscribe(&mut self) -> Subscriber<Message> {
        // Assign a subscriber ID
        let subscriber_id = {
            let mut core    = self.core.lock().unwrap();
            let id          = core.next_subscriber_id;
            core.next_subscriber_id += 1;

            id
        };

        // Create the subscriber core
        let sub_core = SubCore {
            id:                 subscriber_id,
            published:          true,
            waiting:            VecDeque::new(),
            reserved:           0,
            notify_waiting:     vec![],
            notify_ready:       vec![],
            notify_complete:    vec![]
        };

        // The new subscriber needs a reference to the sub_core and the pub_core
        let sub_core = Arc::new(Mutex::new(sub_core));
        let pub_core = Arc::downgrade(&self.core);

        // Register the subscriber with the core, so it will start receiving messages
        {
            let mut core = self.core.lock().unwrap();
            core.subscribers.insert(subscriber_id, Arc::clone(&sub_core));
        }

        // Create the subscriber
        Subscriber::new(pub_core, sub_core)
    }
}

impl<Message> Drop for ExpiringPublisher<Message> {
    fn drop(&mut self) {
        let to_notify = {
            // Lock the core
            let mut pub_core = self.core.lock().unwrap();

            // Check that this is the last publisher on this core
            pub_core.publisher_count -= 1;
            if pub_core.publisher_count == 0 {
                // Mark all the subscribers as unpublished and notify them so that they close
                let mut to_notify = vec![];

                for subscriber in pub_core.subscribers.values() {
                    let mut subscriber = subscriber.lock().unwrap();

                    // Unpublish the subscriber (so that it hits the end of the stream)
                    subscriber.published    = false;
                    subscriber.notify_ready = vec![];

                    // Add to the things to notify once the lock is released
                    to_notify.extend(subscriber.notify_waiting.drain(..));
                }

                // Return the notifications outside of the lock
                to_notify
            } else {
                // This is not the last core
                vec![]
            }
        };

        // Notify any subscribers that are waiting that we're unpublished
        to_notify.into_iter().for_each(|notify| notify.wake());
    }
}

/*
impl<Message: Clone> Sink for ExpiringPublisher<Message> {
    type SinkItem   = Message;
    type SinkError  = ();

    fn start_send(&mut self, item: Message) -> StartSend<Message, ()> {
        // Publish the message to the core
        let notify = { self.core.lock().unwrap().publish_expiring_oldest(&item) };

        if let Some(notify) = notify {
            // Notify all the subscribers that the item has been published
            notify.into_iter().for_each(|notify| notify.notify());

            // Message sent
            Ok(AsyncSink::Ready)
        } else {
            // At least one subscriber has a full queue, so the message could not be sent
            Ok(AsyncSink::NotReady(item))
        }
    }

    fn poll_complete(&mut self) -> Poll<(), ()> {
        if self.core.lock().unwrap().complete() {
            // All subscribers are ready to receive a message
            Ok(Async::Ready(()))
        } else {
            // At least one subscriber has a full buffer
            Ok(Async::NotReady)
        }
    }
}
*/
