use super::subscriber::*;
use super::pubsub_core::*;
use super::message_publisher::*;

use futures::future::{BoxFuture};

use std::sync::*;
use std::collections::{HashMap, VecDeque};

///
/// A single publisher is a publisher that sends each message to only a single subscriber
/// rather than all of them
/// 
/// This is useful for scheduling messages on the first available worker.
/// 
pub struct SinglePublisher<Message> {
    /// The shared core of this publisher
    core: Arc<Mutex<PubCore<Message>>>
}

impl<Message> SinglePublisher<Message> {
    ///
    /// Creates a new single publisher, which will buffer the specified number of messages
    /// 
    pub fn new(buffer_size: usize) -> SinglePublisher<Message> {
        // Create the core
        let core = PubCore {
            next_subscriber_id: 0,
            publisher_count:    1,
            subscribers:        HashMap::new(),
            max_queue_size:     buffer_size
        };

        // Build the publisher itself
        SinglePublisher {
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

        SinglePublisher {
            core:   Arc::clone(&self.core)
        }
    }
}

impl<Message: 'static+Send+Clone> MessagePublisher for SinglePublisher<Message> {
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
        let when_ready  = PubCore::send_single_subscriber(&self.core);

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
}

impl<Message> Drop for SinglePublisher<Message> {
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
