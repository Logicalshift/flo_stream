use super::message_publisher::*;

use futures::future;
use futures::future::{Future};
use futures::task::{Waker, Poll, Context};
use smallvec::*;

use std::pin::*;
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

    /// The IDs of subscribers waiting to be sent messages (used when publishing to single subscribers, to ensure that we don't just fill the queue of the first subscriber)
    pub waiting: Vec<usize>,

    /// The maximum size of queue to allow in any one subscriber
    pub max_queue_size: usize,

    /// Wakers to notify when this core is closed
    pub notify_closed: HashMap<usize, Waker>
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
    pub notify_complete: Vec<Waker>,
}

impl<Message> SubCore<Message> {
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
            let mut sub_core    = arc_self.lock().unwrap();
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
    fn send_single_message(arc_self: &Arc<Mutex<SubCore<Message>>>, message: Message) {
        let waiting_wakers = {
            // Add the message to the waiting list
            let mut sub_core    = arc_self.lock().unwrap();
            sub_core.reserved   -= 1;
            sub_core.waiting.push_back(message);

            // Wake anything that was waiting for a new message
            sub_core.notify_waiting.drain(..).collect::<SmallVec<[_; 8]>>()
        };

        // Notify all of the wakers
        waiting_wakers.into_iter().for_each(|waiting_waker| waiting_waker.wake());
    }
}

impl<Message: Clone> SubCore<Message> {
    ///
    /// Queues a message on this core, returning the list of wakers that need to be notified once the message has finished sending
    ///
    fn send_message(arc_self: &Arc<Mutex<SubCore<Message>>>, message: &Message) -> SmallVec<[Waker; 8]> {
        let waiting_wakers = {
            // Add the message to the waiting list
            let mut sub_core    = arc_self.lock().unwrap();
            sub_core.reserved   -= 1;
            sub_core.waiting.push_back(message.clone());

            // Wake anything that was waiting for a new message
            sub_core.notify_waiting.drain(..).collect::<SmallVec<[_; 8]>>()
        };

        // Result is the list of wakers to be notified
        waiting_wakers
    }
}

impl<Message: 'static+Send> PubCore<Message> {
    ///
    /// Checks the waiting subscriber list for a subscriber ready to receive a message
    ///
    fn poll_waiting_subscribers(&mut self) -> Poll<MessageSender<Message>> {
        while let Some(possible_subscriber) = self.waiting.pop() {
            if let Some(subscriber) = self.subscribers.get(&possible_subscriber) {
                let mut sub_core = subscriber.lock().unwrap();

                if sub_core.queue_size() < self.max_queue_size {
                    // This is the subscriber we're going to send to
                    let subscriber1 = Arc::clone(subscriber);
                    let subscriber2 = Arc::clone(subscriber);

                    // Reserve a slot in the queue
                    sub_core.reserved += 1;

                    // Create the structure that will send the message when it's ready
                    let sender      = MessageSender::new(move |message| {
                        // Send the message to this core
                        SubCore::send_single_message(&subscriber1, message);
                    }, move || {
                        // Cancel the send and allow something else to take the slot
                        SubCore::cancel_send(&subscriber2);
                    });

                    return Poll::Ready(sender);
                }
            }
        }

        // No subscriber was ready
        Poll::Pending
    }

    ///
    /// Fills the list of waiting subscriber IDs
    ///
    fn fill_waiting_subscribers(&mut self) {
        self.waiting = self.subscribers.keys().map(|key| *key).collect();
    }

    ///
    /// Waits for a subscriber to become available and returns a future message sender that will post to that subscriber
    ///
    pub fn send_single_subscriber(arc_self: &Arc<Mutex<PubCore<Message>>>) -> impl Future<Output=MessageSender<Message>>+Send {
        let core = Arc::clone(arc_self);

        future::poll_fn(move |context| {
            // Lock the core and all of the subscribers
            let mut core    = core.lock().unwrap();

            // Check subscribers in the order in 'waiting' (so we send messages evenly to all subscribers)
            if let Poll::Ready(sender) = core.poll_waiting_subscribers() {
                // A waiting subscriber was ready
                Poll::Ready(sender)
            } else {
                // Refill the list and try again (the waiting list may have been partially filled or out-of-date)
                core.fill_waiting_subscribers();

                if let Poll::Ready(sender) = core.poll_waiting_subscribers() {
                    // A waiting subscriber was ready
                    Poll::Ready(sender)
                } else {
                    // All subscribers have full queues

                    // Notify when the subscriber queue becomes empty
                    for (_id, subcore) in core.subscribers.iter() {
                        subcore.lock().unwrap().notify_ready.push(context.waker().clone());
                    }

                    // Wake when a subscriber is available
                    Poll::Pending
                }
            }
        })
    }

    ///
    /// Returns a future that will return when all of the subscribers have no data left to process
    ///
    pub fn when_empty(arc_self: &Arc<Mutex<PubCore<Message>>>) -> impl Future<Output=()>+Send {
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
}

impl<Message: 'static+Send+Clone> PubCore<Message> {
    ///
    /// Waits for all of the subscribers to become available and returns a sender that will send a message to all of them
    /// at once
    ///
    pub fn send_all_subscribers(arc_self: &Arc<Mutex<PubCore<Message>>>) -> impl Future<Output=MessageSender<Message>>+Send {
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
            for (id, _subscriber, sub_core) in subscribers.iter_mut() {
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
                        reserved_ids.insert(*id);
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

            let core                = Arc::clone(&core);
            let sender              = MessageSender::new(move |message| {
                let waiting_wakers = {
                    // Lock the core while we send to the subscribers (so only one sender can be active at once)
                    let _pub_core = core.lock().unwrap();

                    // Send the message via all the subscribers
                    (*all_subscribers1).iter().flat_map(|subscriber| SubCore::send_message(subscriber, &message)).collect::<Vec<_>>()
                };

                waiting_wakers.into_iter().for_each(|waiting_waker| waiting_waker.wake());
            },
            move || { 
                (*all_subscribers2).iter().for_each(|subscriber| SubCore::cancel_send(subscriber));
            });
            
            Poll::Ready(sender)
        })
    }

    ///
    /// Sends to all subscribers, expiring the oldest unsent message
    ///
    pub fn send_all_expiring_oldest(arc_self: &Arc<Mutex<PubCore<Message>>>) -> impl Future<Output=MessageSender<Message>>+Send {
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
            for (id, _subscriber, sub_core) in subscribers.iter_mut() {
                if !reserved_ids.contains(id) {
                    // We haven't already reserved a slot in this queue
                    if sub_core.queue_size() >= pub_core.max_queue_size && sub_core.waiting.len() == 0 {
                        // The queue is full of messages that have not been generated yet. We need to wait for it to become ready or for a
                        // waiting item to be pushed.
                        sub_core.notify_ready.push(context.waker().clone());
                        sub_core.notify_waiting.push(context.waker().clone());
                        return Poll::Pending;
                    } else if sub_core.queue_size() >= pub_core.max_queue_size {
                        // The queue is full but we can discard a message
                        sub_core.waiting.pop_front();

                        // Reserve our space instead
                        sub_core.reserved += 1;
                        reserved_ids.insert(*id);
                    } else {
                        // This subscriber has a slot available: reserve it for us
                        // TODO: if the future is dropped we need to return these reservations to their respective cores
                        sub_core.reserved += 1;
                        reserved_ids.insert(*id);
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
            let core                = Arc::clone(&core);

            let sender              = MessageSender::new(move |message| {
                let waiting_wakers = {
                    // Lock the core while we send to the subscribers (so only one sender can be active at once)
                    let _pub_core = core.lock().unwrap();

                    // Send the message via all the subscribers
                    (*all_subscribers1).iter().flat_map(|subscriber| SubCore::send_message(subscriber, &message)).collect::<Vec<_>>()
                };

                waiting_wakers.into_iter().for_each(|waiting_waker| waiting_waker.wake());
            },
            move || { 
                (*all_subscribers2).iter().for_each(|subscriber| SubCore::cancel_send(subscriber));
            });
            
            Poll::Ready(sender)
        })
    }
}

lazy_static! {
    pub static ref NEXT_CLOSE_FUTURE_ID: Mutex<usize> = Mutex::new(0);
}

///
/// Future that is used to notify when a pubcore is closed
///
pub (crate) struct CoreClosedFuture<Message> {
    /// ID of this future (used to remove the waker if the future is dropped)
    id: usize,

    /// The core that this future belongs to
    core: Weak<Mutex<PubCore<Message>>>
}

impl<Message> CoreClosedFuture<Message> {
    pub (crate) fn new(core: Arc<Mutex<PubCore<Message>>>) -> CoreClosedFuture<Message> {
        // Get an ID for this future
        let next_id = {
            let mut id = NEXT_CLOSE_FUTURE_ID.lock().unwrap();
            let next_id = *id;
            *id += 1;

            next_id
        };

        // The core future maintains a weak reference to the core
        CoreClosedFuture {
            id:     next_id,
            core:   Arc::downgrade(&core)
        }
    }
}

impl<Message> Drop for CoreClosedFuture<Message> {
    fn drop(&mut self) {
        if let Some(core) = self.core.upgrade() {
            core.lock().unwrap().notify_closed.remove(&self.id);
        }
    }
}

impl<Message> Future for CoreClosedFuture<Message> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context) -> Poll<()> {
        let self_ref = self.as_mut();

        if let Some(core) = self_ref.core.upgrade() {
            let mut core = core.lock().unwrap();

            if core.publisher_count == 0 {
                // Core has been closed if the publisher count reaches 0
                Poll::Ready(())
            } else {
                // Wake when the core is closed
                core.notify_closed.insert(self.id, context.waker().clone());
                Poll::Pending
            }
        } else {
            // Core has been closed if the reference count reaches 0
            Poll::Ready(())
        }
    }
}
