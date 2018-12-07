use futures::task;
use futures::task::Task;

use std::sync::*;
use std::collections::{VecDeque, HashMap};

///
/// The shared publisher core, used when subscribers need to send messages to their publisher
/// 
pub (crate) struct PubCore<Message> {
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
pub (crate) struct SubCore<Message> {
    /// Unique ID for the subscriber represented by this core
    pub id: usize,

    /// True while the publisher owning this core is alive
    pub published: bool,

    /// Messages ready to be sent to this core
    pub waiting: VecDeque<Message>,

    /// Notification tasks for when the 'waiting' queue becomes non-empty
    pub notify_waiting: Vec<Task>,

    /// If the publisher is waiting on this subscriber, this is the notification to send
    pub notify_ready: Vec<Task>,

    /// If the publisher is waiting for this subscriber to complete, this is the notification to send
    pub notify_complete: Vec<Task>
}

impl<Message: Clone> PubCore<Message> {
    ///
    /// Attempts to publish a message to all subscribers, returning the list of notifications that need to be generated
    /// if successful, or None if the message could not be sent
    /// 
    pub fn publish(&mut self, message: &Message) -> Option<Vec<Task>> {
        let max_queue_size = self.max_queue_size;
        
        // Lock all of the subscribers
        let mut subscribers = self.subscribers.values()
            .map(|subscriber| subscriber.lock().unwrap())
            .collect::<Vec<_>>();

        // All subscribers must have enough space (we do not queue the message if any subscribe cannot accept it)
        let mut ready = true;
        for subscriber in subscribers.iter_mut() {
            if subscriber.waiting.len() >= max_queue_size {
                // This subscriber needs to notify us when it's ready
                subscriber.notify_ready.push(task::current());

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
}

///
/// Outcome of a single publish request
/// 
pub (crate) enum PublishSingleOutcome<Message> {
    /// Message returned unpublished
    NotPublished(Message),

    /// Message published, with some notifications that need to be fired
    Published(Vec<Task>)
}

impl<Message> PubCore<Message> {
    ///
    /// Attempts to publish a message to all subscribers, returning the list of notifications that need to be generated
    /// if successful, or None if the message could not be sent
    /// 
    pub fn publish_single(&mut self, message: Message) -> PublishSingleOutcome<Message> {
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
            .for_each(|subscriber| subscriber.notify_ready.push(task::current()));

        // Message was not published
        PublishSingleOutcome::NotPublished(message)
    }

    ///
    /// Checks this core for completion. If any messages are still waiting to be processed, returns false and sets the 'notify_complete' task
    /// 
    pub fn complete(&mut self) -> bool {
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
                subscriber.notify_complete.push(task::current());
            }
        }

        complete
    }
}