use super::subscriber::*;
use super::pubsub_core::*;
use super::message_publisher::*;

use futures::future;
use futures::future::{BoxFuture};

use std::sync::*;
use std::collections::{VecDeque};

///
/// A weak publisher is a publisher that 
///
pub struct WeakPublisher<Message> {
    /// The shared core of this publisher
    pub (super) core: Weak<Mutex<PubCore<Message>>>
}

impl<Message: Clone> WeakPublisher<Message> {
    ///
    /// Counts the number of subscribers in this publisher
    /// 
    pub fn count_subscribers(&self) -> usize {
        self.core.upgrade()
            .map(|core| core.lock().unwrap().subscribers.len())
            .unwrap_or(0)
    }

    ///
    /// Creates a duplicate publisher that can be used to publish to the same streams as this object
    /// 
    pub fn republish(&self) -> Self {
        WeakPublisher {
            core:   Weak::clone(&self.core)
        }
    }
}

impl<Message: 'static+Send+Clone> MessagePublisher for WeakPublisher<Message> {
    type Message = Message;

    ///
    /// Subscribes to this publisher
    /// 
    /// Subscribers only receive messages sent to the publisher after they are created.
    /// 
    fn subscribe(&mut self) -> Subscriber<Message> {
        let core = self.core.upgrade();

        if let Some(core) = core {
            // Assign a subscriber ID
            let subscriber_id = {
                let mut core    = core.lock().unwrap();
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
            let pub_core = Arc::downgrade(&core);

            // Register the subscriber with the core, so it will start receiving messages
            {
                let mut core = core.lock().unwrap();
                core.subscribers.insert(subscriber_id, Arc::clone(&sub_core));
            }

            // Create the subscriber
            Subscriber::new(pub_core, sub_core)
        } else {
            // Create a subscriber that is already closed
            let sub_core = SubCore {
                id:                 0,
                published:          true,
                waiting:            VecDeque::new(),
                reserved:           0,
                notify_waiting:     vec![],
                notify_ready:       vec![],
                notify_complete:    vec![]
            };

            Subscriber::new(Weak::default(), Arc::new(Mutex::new(sub_core)))
        }
    }

    ///
    /// Reserves a space for a message with the subscribers, returning when it's ready
    ///
    fn when_ready(&mut self) -> BoxFuture<'static, MessageSender<Message>> {
        let core = self.core.upgrade();

        if let Some(core) = core {
            let when_ready  = PubCore::send_all_subscribers(&core);

            Box::pin(when_ready)
        } else {
            Box::pin(future::ready(MessageSender::new(|_msg| {}, || {})))
        }
    }

    ///
    /// Waits until all subscribers have consumed all pending messages
    ///
    fn when_empty(&mut self) -> BoxFuture<'static, ()> {
        let core = self.core.upgrade();

        if let Some(core) = core {
            let when_empty  = PubCore::when_empty(&core);

            Box::pin(when_empty)
        } else {
            Box::pin(future::ready(()))
        }
    }
}
