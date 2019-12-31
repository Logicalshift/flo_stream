extern crate futures;
extern crate flo_stream;

use ::desync::*;
use flo_stream::*;

use futures::*;
use futures::stream;
use futures::executor;
use futures::task::{ArcWake};
use futures::channel::mpsc;

use std::mem;
use std::thread;
use std::sync::*;
use std::sync::mpsc::channel;

#[derive(Clone)]
struct NotifyNothing;

impl ArcWake for NotifyNothing {
    fn wake_by_ref(_arc_self: &Arc<Self>) { }
}

#[test]
fn receive_on_one_subscriber() {
    let mut publisher   = Publisher::<i32>::new(10);
    let mut subscriber  = publisher.subscribe();

    executor::block_on(async {
        publisher.publish(1).await;
        publisher.publish(2).await;
        publisher.publish(3).await;

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));
    })
}

#[test]
fn republish_3_times() {
    let mut publisher1  = Publisher::new(10);
    let mut publisher2  = publisher1.republish();
    let mut publisher3  = publisher2.republish();
    let mut subscriber  = publisher1.subscribe();

    executor::block_on(async {
        publisher1.publish(1).await;
        publisher2.publish(2).await;
        publisher3.publish(3).await;

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));
    })
}

#[test]
fn complete_when_empty() {
    let mut publisher   = Publisher::new(10);
    let mut subscriber  = publisher.subscribe();

    executor::block_on(async {
        publisher.when_empty().await;

        publisher.publish(1).await;
        publisher.publish(2).await;
        publisher.publish(3).await;

        let mut not_empty   = publisher.when_empty();
        let waker           = task::waker(Arc::new(NotifyNothing));
        let mut ctxt        = task::Context::from_waker(&waker);
        assert!(not_empty.poll_unpin(&mut ctxt) == task::Poll::Pending);

        let mut not_empty   = publisher.when_empty();

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));

        assert!(not_empty.poll_unpin(&mut ctxt) == task::Poll::Ready(()));
    });
}

#[test]
fn subscriber_closes_when_publisher_closes() {
    let mut closed_subscriber = {
        let mut publisher   = Publisher::new(10);
        let mut subscriber  = publisher.subscribe();

        executor::block_on(async {
            publisher.publish(1).await;
            assert!(subscriber.next().await == Some(1));
        });

        subscriber
    };

    executor::block_on(async {
        assert!(closed_subscriber.next().await == None);
    });
}

#[test]
fn read_on_multiple_subscribers() {
    let mut publisher       = Publisher::new(10);
    let mut subscriber1     = publisher.subscribe();
    let mut subscriber2     = publisher.subscribe();

    executor::block_on(async {
        let waker           = task::waker(Arc::new(NotifyNothing));
        let mut ctxt        = task::Context::from_waker(&waker);

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Ready(()));

        publisher.publish(1).await;
        publisher.publish(2).await;
        publisher.publish(3).await;

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Pending);

        assert!(subscriber1.next().await == Some(1));
        assert!(subscriber1.next().await == Some(2));
        assert!(subscriber1.next().await == Some(3));

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Pending);

        assert!(subscriber2.next().await == Some(1));
        assert!(subscriber2.next().await == Some(2));
        assert!(subscriber2.next().await == Some(3));

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Ready(()));
    })
}

#[test]
fn clone_subscribers() {
    let mut publisher   = Publisher::new(10);
    let mut subscriber1 = publisher.subscribe();

    executor::block_on(async {
        let waker           = task::waker(Arc::new(NotifyNothing));
        let mut ctxt        = task::Context::from_waker(&waker);

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Ready(()));

        publisher.publish(1).await;
        publisher.publish(2).await;
        publisher.publish(3).await;

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Pending);

        let mut subscriber2 = subscriber1.clone();

        assert!(subscriber1.next().await == Some(1));
        assert!(subscriber1.next().await == Some(2));
        assert!(subscriber1.next().await == Some(3));

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Pending);

        assert!(subscriber2.next().await == Some(1));
        assert!(subscriber2.next().await == Some(2));
        assert!(subscriber2.next().await == Some(3));

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Ready(()));
    });
}

#[test]
fn clone_subscribers_after_reading() {
    let mut publisher   = Publisher::new(10);
    let mut subscriber1 = publisher.subscribe();

    executor::block_on(async {
        let waker           = task::waker(Arc::new(NotifyNothing));
        let mut ctxt        = task::Context::from_waker(&waker);

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Ready(()));

        publisher.publish(1).await;
        publisher.publish(2).await;
        publisher.publish(3).await;

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Pending);

        assert!(subscriber1.next().await == Some(1));
        assert!(subscriber1.next().await == Some(2));

        let mut subscriber2 = subscriber1.clone();
        assert!(subscriber1.next().await == Some(3));

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Pending);

        assert!(subscriber2.next().await == Some(3));

        assert!(publisher.when_empty().poll_unpin(&mut ctxt) == task::Poll::Ready(()));
    });
}

#[test]
fn complete_on_multiple_subscribers() {
    let mut publisher   = Publisher::new(10);
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
    })
}

#[test]
fn skip_messages_sent_before_subscription() {
    let mut publisher       = Publisher::new(10);

    executor::block_on(async {
        publisher.publish(1).await;
        let mut subscriber1 = publisher.subscribe();
        publisher.publish(2).await;
        let mut subscriber2 = publisher.subscribe();
        publisher.publish(3).await;

        assert!(subscriber1.next().await == Some(2));
        assert!(subscriber1.next().await == Some(3));

        assert!(subscriber2.next().await == Some(3));
    });
}

#[test]
fn blocks_if_subscribers_are_full() {
    let mut publisher   = Publisher::new(10);
    let mut subscriber  = publisher.subscribe();

    let waker           = task::waker(Arc::new(NotifyNothing));
    let mut ctxt        = task::Context::from_waker(&waker);

    let mut send_count  = 0;
    while send_count < 100 {
        match publisher.publish(send_count).poll_unpin(&mut ctxt) {
            task::Poll::Pending => {
                break;
            },

            _ => { }
        }

        send_count += 1;
    }

    assert!(send_count == 10);

    executor::block_on(async {
        for a in 0..10 {
            assert!(subscriber.next().await == Some(a));
        }
    })
}

#[test]
fn single_receive_on_one_subscriber() {
    let mut publisher   = SinglePublisher::new(10);
    let mut subscriber  = publisher.subscribe();

    executor::block_on(async {
        publisher.publish(1).await;
        publisher.publish(2).await;
        publisher.publish(3).await;

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));
    });
}

#[test]
fn single_receive_on_two_subscribers() {
    let (result_tx, result_rx) = channel();

    let mut publisher   = SinglePublisher::<i32>::new(1);
    let mut subscriber1 = publisher.subscribe();
    let mut subscriber2 = publisher.subscribe();

    // Subscribers will block us if there's nothing listening, so spawn two threads to receive data
    let result = result_tx.clone();
    thread::spawn(move || {
        executor::block_on(async {
            result.send(subscriber1.next().await).unwrap();
        })
    });

    let result = result_tx.clone();
    thread::spawn(move || {
        executor::block_on(async {
            result.send(subscriber2.next().await).unwrap();
        })
    });

    executor::block_on(async {
        publisher.publish(1).await;
        publisher.publish(2).await;

        // Read the results from the threads (they're unordered, but as each thread only reads a single value we'll see if it gets sent to multiple places)
        let msg1 = result_rx.recv().unwrap();
        let msg2 = result_rx.recv().unwrap();

        // Should be 1, 2 in either order
        assert!(msg1 == Some(1) || msg1 == Some(2));

        if msg1 == Some(1) {
            assert!(msg2 == Some(2));
        } else {
            assert!(msg2 == Some(1));
        }
    });
}

#[test]
fn expiring_removes_oldest_events_first() {
    let mut publisher   = ExpiringPublisher::new(5);
    let mut subscriber  = publisher.subscribe();

    executor::block_on(async {
        for evt in 0..100 {
            publisher.publish(evt).await;
        }

        assert!(subscriber.next().await == Some(95));
        assert!(subscriber.next().await == Some(96));
        assert!(subscriber.next().await == Some(97));
        assert!(subscriber.next().await == Some(98));
        assert!(subscriber.next().await == Some(99));
    })
}

#[test]
fn send_all_stream() {
    let mut publisher   = Publisher::<i32>::new(10);
    let mut subscriber  = publisher.subscribe();

    executor::block_on(async {
        let stream = stream::iter(vec![1, 2, 3]);
        publisher.send_all(stream).await;

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));
    })
}

#[test]
fn drop_publisher_after_send_all() {
    let mut publisher   = Publisher::<i32>::new(10);
    let mut subscriber  = publisher.subscribe();

    executor::block_on(async {
        let stream = stream::iter(vec![1, 2, 3]);
        publisher.send_all(stream).await;
        mem::drop(publisher);

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));
        assert!(subscriber.next().await == None);
    })
}

#[test]
fn drop_publisher_with_weak_publisher_after_publish() {
    let mut publisher       = Publisher::<i32>::new(10);
    let mut weak_publisher  = publisher.republish_weak();
    let mut subscriber      = publisher.subscribe();

    executor::block_on(async {
        weak_publisher.publish(1).await;
        weak_publisher.publish(2).await;
        weak_publisher.publish(3).await;
        mem::drop(publisher);

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));
        assert!(subscriber.next().await == None);

        weak_publisher.publish(4).await;
    })
}

#[test]
fn drop_publisher_with_weak_publisher_after_send_all() {
    let mut publisher       = Publisher::<i32>::new(10);
    let mut weak_publisher  = publisher.republish_weak();
    let mut subscriber      = publisher.subscribe();

    executor::block_on(async {
        let stream = stream::iter(vec![1, 2, 3]);
        publisher.send_all(stream).await;
        mem::drop(publisher);

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));
        assert!(subscriber.next().await == None);

        weak_publisher.publish(4).await;
    })
}

#[test]
fn send_all_drops_stream_when_publisher_dropped() {
    use std::time::Duration;

    let mut publisher       = Publisher::<i32>::new(10);
    let mut weak_publisher  = publisher.republish_weak();
    let mut subscriber      = publisher.subscribe();
    let sender              = Desync::new(());

    executor::block_on(async {
        let (mut tx, rx) = mpsc::channel(1);

        // Create a future to send all of the data
        sender.desync(move |_| {
            executor::block_on(weak_publisher.send_all(rx));
        });

        // Send a value while the publisher exists
        tx.send(1).await.unwrap();
        assert!(subscriber.next().await == Some(1));

        // Drop the publisher
        mem::drop(publisher);

        // Give the sending thread a chance to shut the stream down 
        thread::sleep(Duration::from_millis(20));

        // Should not be able to send any more as only the weak publisher still exists
        assert!(tx.send(2).await.is_err());
    })
}
