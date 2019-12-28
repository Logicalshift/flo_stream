use super::publisher_sink::*;

use futures::future;
use futures::future::{Future};
use futures::task;
use futures::task::{Waker, Poll};
use smallvec::*;

use std::sync::*;
use std::collections::{VecDeque, HashMap, HashSet};

///
/// The shared publisher core, used when subscribers need to send messages to their publisher
/// 
pub (super) struct PubCore<Message> {
    /// The number of publishers using this core
    pub publisher_count: usize,

    /// The next ID to assign to a new subscriber
    pub next_subscriber_id: usize,

    /// The subscribers to this publisher
    pub subscribers: HashMap<usize, Arc<Mutex<SubCore<Message>>>>,

    /// The maximum size of queue to allow in any one subscriber
    pub max_queue_size: usize,
}

///
/// The core shared structure between a publisher and subscriber
/// 
pub (super) struct SubCore<Message> {
    /// Unique ID for the subscriber represented by this core
    pub id: usize,

    /// True while the publisher owning this core is alive
    pub published: bool,

    /// The number of reserved slots in the waiting list (that is, have an assigned message sender)
    pub reserved: usize,

    /// Messages ready to be sent to this core
    pub waiting: VecDeque<Message>,

    /// Notification tasks for when the 'waiting' queue becomes non-empty
    pub notify_waiting: Vec<Waker>,

    /// If the publisher is waiting on this subscriber, this is the notification to send
    pub notify_ready: Vec<Waker>,

    /// If the publisher is waiting for this subscriber to complete, this is the notification to send
    pub notify_complete: Vec<Waker>
}

impl<Message: Clone> SubCore<Message> {
    ///
    /// Returns the current size of the queue in this subscriber
    ///
    fn queue_size(&self) -> usize {
        self.waiting.len() + self.reserved
    }

    ///
    /// For a sender that's reserved a slot in this subscriber, returns the slot to the pool
    ///
    fn cancel_send(arc_self: &Arc<Mutex<SubCore<Message>>>) {
        let ready_wakers = {
            // Add the message to the waiting list
            let sub_core        = arc_self.lock().unwrap();
            sub_core.reserved   -= 1;

            // Wake anything that was waiting for a new message
            sub_core.notify_ready.drain(..).collect::<SmallVec<[_; 8]>>()
        };

        // Notify anything that was waiting for this subscriber to become ready
        ready_wakers.into_iter().for_each(|ready_waker| ready_waker.wake());
    }

    ///
    /// Sends a message to this core, reducing the reserved count and notifying anything that's waiting for the core to wake up
    ///
    fn send_message(arc_self: &Arc<Mutex<SubCore<Message>>>, message: &Message) {
        let waiting_wakers = {
            // Add the message to the waiting list
            let sub_core        = arc_self.lock().unwrap();
            sub_core.reserved   -= 1;
            sub_core.waiting.push_back(message.clone());

            // Wake anything that was waiting for a new message
            sub_core.notify_waiting.drain(..).collect::<SmallVec<[_; 8]>>()
        };

        // Notify all of the wakers
        waiting_wakers.into_iter().for_each(|waiting_waker| waiting_waker.wake());
    }
}

///
/// Outcome of a single publish request
/// 
pub (crate) enum PublishSingleOutcome<Message> {
    /// Message returned unpublished
    NotPublished(Message),

    /// Message published, with some notifications that need to be fired
    Published(Vec<Waker>)
}

impl<Message: 'static+Clone+Send> PubCore<Message> {
    ///
    /// Waits for a subscriber to become available and returns a future message sender that will post to that subscriber
    ///
    pub fn send_single_subscriber(arc_self: &Arc<Mutex<PubCore<Message>>>) -> impl Future<Output=MessageSender<Message>> {
        let core = Arc::clone(arc_self);

        future::poll_fn(move |context| {
            // Lock the core and all of the subscribers
            let core            = core.lock().unwrap();
            let mut subscribers = core.subscribers.iter()
                .map(|(id, subscriber)| {
                    (*id, subscriber, subscriber.lock().unwrap())
                })
                .collect::<SmallVec<[_; 8]>>();

            // Check for a subscriber with a free slot
            for (_id, subscriber, sub_core) in subscribers.iter_mut() {
                if sub_core.queue_size() < core.max_queue_size {
                    // This is the subscriber we're going to send to
                    let subscriber1 = Arc::clone(subscriber);
                    let subscriber2 = Arc::clone(subscriber);

                    // Reserve a slot in the queue
                    sub_core.reserved += 1;

                    // Create the structure that will send the message when it's ready
                    let sender      = MessageSender::new(move |message| {
                        // Send the message to this core
                        SubCore::send_message(&subscriber1, &message);
                    }, move || {
                        // Cancel the send and allow something else to take the slot
                        SubCore::cancel_send(&subscriber2);
                    });

                    return Poll::Ready(sender);
                }
            }

            // Have the subscribers notify us when a slot becomes free
            subscribers.iter_mut().map(|(_id, _subscriber, sub_core)| { sub_core.notify_ready.push(context.waker().clone()) });

            // Pending on a subscriber becoming ready
            Poll::Pending
        })
    }

    ///
    /// Waits for all of the subscribers to become available and returns a sender that will send a message to all of them
    /// at once
    ///
    pub fn send_all_subscribers(arc_self: &Arc<Mutex<PubCore<Message>>>) -> impl Future<Output=MessageSender<Message>> {
        let core                = Arc::clone(arc_self);
        let mut reserved_ids    = HashSet::new();

        future::poll_fn(move |context| {
            // Lock the core and all of the subscribers
            let core            = Arc::clone(&core);
            let pub_core        = core.lock().unwrap();
            let mut subscribers = pub_core.subscribers.iter()
                .map(|(id, subscriber)| {
                    (*id, subscriber, subscriber.lock().unwrap())
                })
                .collect::<SmallVec<[_; 8]>>();

            // Check that all subscribers have a free slot
            for (id, subscriber, sub_core) in subscribers.iter_mut() {
                if !reserved_ids.contains(id) {
                    // We haven't already reserved a slot in this queue
                    if sub_core.queue_size() >= pub_core.max_queue_size {
                        // The queue is full: we need to wait for this subscriber to have a slot ready
                        sub_core.notify_ready.push(context.waker().clone());
                        return Poll::Pending;
                    } else {
                        // This subscriber has a slot available: reserve it for us
                        // TODO: if the future is dropped we need to return these reservations to their respective cores
                        sub_core.reserved += 1;
                        reserved_ids.insert(id);
                    }
                }
            }

            // All subscribers have a slot reserved for this message: create the sender
            // In the event a new subscriber is created between the future completing and the sender being notified of its message
            // we will not send the message to that subscriber
            let all_subscribers     = subscribers.iter().map(|(_, subscriber, _)| Arc::clone(subscriber));
            let all_subscribers     = all_subscribers.collect::<SmallVec<[_; 8]>>();
            let all_subscribers     = Arc::new(all_subscribers);
            let all_subscribers1    = all_subscribers;
            let all_subscribers2    = Arc::clone(&all_subscribers1);

            let sender              = MessageSender::new(move |message| {
                // Lock the core while we send to the subscribers (so only one sender can be active at once)
                let _pub_core = core.lock().unwrap();

                // Send the message via all the subscribers
                (*all_subscribers1).iter().for_each(|subscriber| SubCore::send_message(subscriber, &message));
            },
            move || { 
                (*all_subscribers2).iter().for_each(|subscriber| SubCore::cancel_send(subscriber));
            });
            
            Poll::Ready(sender)
        })
    }

    ///
    /// Returns a future that will return when all of the subscribers have no data left to process
    ///
    pub fn when_empty(arc_self: &Arc<Mutex<PubCore<Message>>>) -> impl Future<Output=()> {
        let core                = Arc::clone(arc_self);

        future::poll_fn(move |context| {
            // Lock the core and all of the subscribers
            let core            = Arc::clone(&core);
            let pub_core        = core.lock().unwrap();
            let mut subscribers = pub_core.subscribers.iter()
                .map(|(id, subscriber)| {
                    (*id, subscriber, subscriber.lock().unwrap())
                })
                .collect::<SmallVec<[_; 8]>>();

            // Check that all subscribers are empty (wait on the first that's not)
            for (_id, _subscriber, sub_core) in subscribers.iter_mut() {
                if sub_core.queue_size() > 0 {
                    // Wake when this subscriber becomes ready to check again
                    sub_core.notify_ready.push(context.waker().clone());

                    // Wait for this subscriber to empty
                    return Poll::Pending;
                }
            }

            // All subscribers are empty
            Poll::Ready(())
        })
    }

    ///
    /// Attempts to publish a message to all subscribers, returning the list of notifications that need to be generated
    /// if successful, or None if the message could not be sent
    /// 
    pub fn publish(&mut self, message: &Message, context: &task::Context) -> Option<Vec<Waker>> {
        let max_queue_size = self.max_queue_size;
        
        // Lock all of the subscribers
        let mut subscribers = self.subscribers.values()
            .map(|subscriber| subscriber.lock().unwrap())
            .collect::<Vec<_>>();

        // All subscribers must have enough space (we do not queue the message if any subscriber cannot accept it)
        let mut ready = true;
        for subscriber in subscribers.iter_mut() {
            if subscriber.waiting.len() >= max_queue_size {
                // This subscriber needs to notify us when it's ready
                subscriber.notify_ready.push(context.waker().clone());

                // Not ready
                ready = false;
            }
        }

        if !ready {
            // At least one subscriber has a full queue
            None
        } else {
            // Send to all of the subscribers
            subscribers.iter_mut().for_each(|subscriber| subscriber.waiting.push_back(message.clone()));

            // Claim all of the notifications
            let notifications = subscribers.iter_mut()
                .flat_map(|subscriber| subscriber.notify_waiting.drain(..))
                .collect::<Vec<_>>();

            // Result is the notifications to fire
            Some(notifications)
        }
    }

    ///
    /// Removes the oldest message from any subscribers that are full and then attempts to publish new message.
    /// 
    pub fn publish_expiring_oldest(&mut self, message: &Message, context: &task::Context) -> Option<Vec<Waker>> {
        {
            let max_queue_size = self.max_queue_size;
            
            // Lock all of the subscribers
            let mut subscribers = self.subscribers.values()
                .map(|subscriber| subscriber.lock().unwrap())
                .collect::<Vec<_>>();

            // Expire messages from any subscribers with a full queue
            for subscriber in subscribers.iter_mut() {
                if subscriber.waiting.len() >= max_queue_size {
                    subscriber.waiting.pop_front();
                }
            }
        }

        // Publish the message
        self.publish(message, context)
    }

    ///
    /// Attempts to publish a message to a single subscriber, returning the list of notifications that need to be generated
    /// if successful, or None if the message could not be sent
    /// 
    pub fn publish_single(&mut self, message: Message, context: &task::Context) -> PublishSingleOutcome<Message> {
        let max_queue_size = self.max_queue_size;
        
        // Lock all of the subscribers
        let mut subscribers = self.subscribers.values()
            .map(|subscriber| subscriber.lock().unwrap())
            .collect::<Vec<_>>();

        // Try to find an idle subscriber (one where notify_waiting has a value, or which has more than one free slot in the queue)
        {
            let idle_subscriber = subscribers.iter_mut()
            .filter(|subscriber| subscriber.notify_waiting.len() > 0 || subscriber.waiting.len() < max_queue_size-1)
            .nth(0);

            if let Some(idle_subscriber) = idle_subscriber  {
                // Found an idle subscriber to notify
                let notify = idle_subscriber.notify_waiting.drain(..).collect();

                // Send the message to this subscriber alone
                idle_subscriber.waiting.push_back(message);

                // Caller should notify the subscriber that new data is available
                return PublishSingleOutcome::Published(notify);
            }
        }

        // No idle subscribers. All subscribers should notify us when they're ready
        subscribers.iter_mut()
            .for_each(|subscriber| subscriber.notify_ready.push(context.waker().clone()));

        // Message was not published
        PublishSingleOutcome::NotPublished(message)
    }

    ///
    /// Checks this core for completion. If any messages are still waiting to be processed, returns false and sets the 'notify_complete' task
    /// 
    pub fn complete(&mut self, context: &task::Context) -> bool {
        // The core is ready if there are currently no subscribers with any waiting messages

        // Collect the subscribers into one place
        let mut subscribers = self.subscribers.values()
            .map(|subscriber| subscriber.lock().unwrap())
            .collect::<Vec<_>>();

        // Determine if we're complete or not
        let mut complete = true;
        for subscriber in subscribers.iter_mut() {
            if subscriber.waiting.len() > 0 {
                // Not compelte
                complete = false;

                // This subscriber needs to notify this task when it becomes ready
                subscriber.notify_complete.push(context.waker().clone());
            }
        }

        complete
    }
}