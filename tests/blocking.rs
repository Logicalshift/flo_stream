extern crate futures;
extern crate flo_stream;

use flo_stream::*;

use futures::*;
use futures::executor;
use futures::task::{ArcWake, Poll};

use std::thread;
use std::sync::*;
use std::time::Duration;

#[derive(Clone)]
struct NotifyNothing;

impl ArcWake for NotifyNothing {
    fn wake_by_ref(_arc_self: &Arc<Self>) { }
}

#[test]
pub fn blocks_if_there_are_no_subscribers() {
    let waker           = task::waker(Arc::new(NotifyNothing));
    let mut ctxt        = task::Context::from_waker(&waker);

    let mut publisher   = BlockingPublisher::new(2, 10);

    assert!(publisher.publish(1).poll_unpin(&mut ctxt) == Poll::Pending);
}

#[test]
pub fn blocks_if_there_are_insufficient_subscribers() {
    let waker           = task::waker(Arc::new(NotifyNothing));
    let mut ctxt        = task::Context::from_waker(&waker);

    let mut publisher   = BlockingPublisher::new(2, 10);
    let _subscriber     = publisher.subscribe();

    assert!(publisher.publish(1).poll_unpin(&mut ctxt) == Poll::Pending);
}

#[test]
pub fn unblocks_if_there_are_sufficient_subscribers() {
    let waker           = task::waker(Arc::new(NotifyNothing));
    let mut ctxt        = task::Context::from_waker(&waker);

    let mut publisher   = BlockingPublisher::new(2, 10);
    let _subscriber1    = publisher.subscribe();
    let _subscriber2    = publisher.subscribe();

    assert!(publisher.publish(1).poll_unpin(&mut ctxt) == Poll::Ready(()));
}

#[test]
pub fn read_from_subscribers() {
    let mut publisher   = BlockingPublisher::new(2, 10);
    let mut subscriber1 = publisher.subscribe();
    let mut subscriber2 = publisher.subscribe();

    executor::block_on(async {
        publisher.publish(1).await;
        publisher.publish(2).await;
        publisher.publish(3).await;

        assert!(subscriber1.next().await == Some(1));
        assert!(subscriber1.next().await == Some(2));
        assert!(subscriber1.next().await == Some(3));

        assert!(subscriber2.next().await == Some(1));
        assert!(subscriber2.next().await == Some(2));
        assert!(subscriber2.next().await == Some(3));
    });
}

#[test]
pub fn read_from_thread() {
    let mut subscriber = {
        // Create a shared publisher
        let publisher = BlockingPublisher::new(1, 1);
        let publisher = Arc::new(Mutex::new(publisher));

        // Create a thread to publish some values
        let thread_publisher = publisher.clone();
        thread::spawn(move || {
            let wait_for_subscribers = thread_publisher.lock().unwrap().when_ready();

            executor::block_on(async {
                wait_for_subscribers.await;

                thread_publisher.lock().unwrap().publish(1).await;
                thread_publisher.lock().unwrap().publish(2).await;
                thread_publisher.lock().unwrap().publish(3).await;
            });
        });

        // Pause for a bit to let the thread get ahead of us
        thread::sleep(Duration::from_millis(20));

        // Subscribe to the thread (which should now wake up)
        let subscriber = publisher.lock().unwrap().subscribe();
        subscriber
    };

    // Should receive the values from the thread
    executor::block_on(async {
        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));

        // As we don't retain the publisher, the thread is its only owner. When it finishes, the stream should close.
        assert!(subscriber.next().await == None);
    });
}

#[test]
pub fn read_from_thread_late_start() {
    let mut subscriber = {
        // Create a shared publisher
        let publisher = BlockingPublisher::new(1, 1);
        let publisher = Arc::new(Mutex::new(publisher));

        // Create a thread to publish some values
        let thread_publisher = publisher.clone();
        thread::spawn(move || {
            // Wait for the subscriber to be created
            thread::sleep(Duration::from_millis(20));

            let wait_for_subscribers = thread_publisher.lock().unwrap().when_ready();
            executor::block_on(async {
                wait_for_subscribers.await;

                thread_publisher.lock().unwrap().publish(1).await;
                thread_publisher.lock().unwrap().publish(2).await;
                thread_publisher.lock().unwrap().publish(3).await;
            });
        });

        // Subscribe to the thread (which should now wake up)
        let subscriber = publisher.lock().unwrap().subscribe();
        subscriber
    };

    // Should receive the values from the thread
    executor::block_on(async {
        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));

        // As we don't retain the publisher, the thread is its only owner. When it finishes, the stream should close.
        assert!(subscriber.next().await == None);
    });
}
