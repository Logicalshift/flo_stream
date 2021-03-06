use super::subscriber::*;
use super::pubsub_core::*;
use super::weak_publisher::*;
use super::message_publisher::*;

use futures::future::{BoxFuture};

use std::sync::*;
use std::collections::{HashMap, VecDeque};

///
/// A publisher represents a sink that sends messages to zero or more subscribers
/// 
/// Call `subscribe()` to create subscribers. Any messages sent to this sink will be relayed to all connected
/// subscribers. If the publisher is dropped, any connected subscribers will relay all sent messages and then
/// indicate that they have finished.
/// 
pub struct Publisher<Message> {
    /// The shared core of this publisher
    core: Arc<Mutex<PubCore<Message>>>
}

impl<Message: Clone> Publisher<Message> {
    ///
    /// Creates a new publisher with a particular buffer size
    /// 
    pub fn new(buffer_size: usize) -> Publisher<Message> {
        // Create the core
        let core = PubCore {
            next_subscriber_id: 0,
            publisher_count:    1,
            subscribers:        HashMap::new(),
            notify_closed:      HashMap::new(),
            waiting:            vec![],
            max_queue_size:     buffer_size
        };

        // Build the publisher itself
        Publisher {
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

        Publisher {
            core:   Arc::clone(&self.core)
        }
    }

    ///
    /// Creates a duplicate publisher that can be used to publish to the same streams as this object
    /// 
    /// This creates a 'weak' publisher, which will stop republishing once all of the 'strong' publishers have been dropped.
    ///
    pub fn republish_weak(&self) -> WeakPublisher<Message> {
        WeakPublisher {
            core:   Arc::downgrade(&self.core)
        }
    }
}

impl<Message: 'static+Send+Clone> MessagePublisher for Publisher<Message> {
    type Message = Message;

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

    ///
    /// Reserves a space for a message with the subscribers, returning when it's ready
    ///
    fn when_ready(&mut self) -> BoxFuture<'static, MessageSender<Message>> {
        let when_ready  = PubCore::send_all_subscribers(&self.core);

        Box::pin(when_ready)
    }

    ///
    /// Waits until all subscribers have consumed all pending messages
    ///
    fn when_empty(&mut self) -> BoxFuture<'static, ()> {
        let when_empty  = PubCore::when_empty(&self.core);

        Box::pin(when_empty)
    }

    ///
    /// Returns true if this publisher is closed (will not publish any further messages to its subscribers)
    ///
    fn is_closed(&self) -> bool { false }

    ///
    /// Future that returns when this publisher is closed
    ///
    fn when_closed(&self) -> BoxFuture<'static, ()> {
        Box::pin(CoreClosedFuture::new(Arc::clone(&self.core)))
    }
}

impl<Message> Drop for Publisher<Message> {
    fn drop(&mut self) {
        let to_notify = {
            // Lock the core
            let mut pub_core = self.core.lock().unwrap();

            // Check that this is the last publisher on this core
            pub_core.publisher_count -= 1;
            if pub_core.publisher_count == 0 {
                // Mark all the subscribers as unpublished and notify them so that they close
                let mut to_notify = pub_core.notify_closed.drain()
                    .map(|(_id, waker)| waker)
                    .collect::<Vec<_>>();

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
