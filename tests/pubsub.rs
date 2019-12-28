extern crate futures;
extern crate flo_stream;

use flo_stream::*;

use futures::*;
use futures::executor;

use std::thread;
use std::sync::mpsc::channel;

/*
#[derive(Clone)]
struct NotifyNothing;

impl Notify for NotifyNothing {
    fn notify(&self, _: usize) { }
}
*/

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

/*
#[test]
fn complete_when_empty() {
    let mut publisher   = Publisher::new(10);
    let subscriber      = publisher.subscribe();

    let mut publisher   = executor::spawn(publisher);
    let mut subscriber  = executor::spawn(subscriber);

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::Ready(())));

    publisher.wait_send(1).unwrap();
    publisher.wait_send(2).unwrap();
    publisher.wait_send(3).unwrap();

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::NotReady));

    assert!(subscriber.wait_stream() == Some(Ok(1)));
    assert!(subscriber.wait_stream() == Some(Ok(2)));
    assert!(subscriber.wait_stream() == Some(Ok(3)));

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::Ready(())));
}

#[test]
fn subscriber_closes_when_publisher_closes() {
    let mut closed_subscriber = {
        let mut publisher   = Publisher::new(10);
        let subscriber      = publisher.subscribe();

        let mut publisher   = executor::spawn(publisher);
        let mut subscriber  = executor::spawn(subscriber);

        publisher.wait_send(1).unwrap();

        assert!(subscriber.wait_stream() == Some(Ok(1)));
        subscriber
    };

    assert!(closed_subscriber.wait_stream() == None);
}

#[test]
fn read_on_multiple_subscribers() {
    let mut publisher   = Publisher::new(10);
    let subscriber1     = publisher.subscribe();
    let subscriber2     = publisher.subscribe();

    let mut publisher   = executor::spawn(publisher);
    let mut subscriber1 = executor::spawn(subscriber1);
    let mut subscriber2 = executor::spawn(subscriber2);

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::Ready(())));

    publisher.wait_send(1).unwrap();
    publisher.wait_send(2).unwrap();
    publisher.wait_send(3).unwrap();

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::NotReady));

    assert!(subscriber1.wait_stream() == Some(Ok(1)));
    assert!(subscriber1.wait_stream() == Some(Ok(2)));
    assert!(subscriber1.wait_stream() == Some(Ok(3)));

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::NotReady));

    assert!(subscriber2.wait_stream() == Some(Ok(1)));
    assert!(subscriber2.wait_stream() == Some(Ok(2)));
    assert!(subscriber2.wait_stream() == Some(Ok(3)));

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::Ready(())));
}

#[test]
fn clone_subscribers() {
    let mut publisher   = Publisher::new(10);
    let subscriber1     = publisher.subscribe();

    let mut publisher   = executor::spawn(publisher);
    let mut subscriber1 = executor::spawn(subscriber1);

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::Ready(())));

    publisher.wait_send(1).unwrap();
    publisher.wait_send(2).unwrap();
    publisher.wait_send(3).unwrap();

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::NotReady));

    let mut subscriber2 = executor::spawn(subscriber1.get_ref().clone());

    assert!(subscriber1.wait_stream() == Some(Ok(1)));
    assert!(subscriber1.wait_stream() == Some(Ok(2)));
    assert!(subscriber1.wait_stream() == Some(Ok(3)));

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::NotReady));

    assert!(subscriber2.wait_stream() == Some(Ok(1)));
    assert!(subscriber2.wait_stream() == Some(Ok(2)));
    assert!(subscriber2.wait_stream() == Some(Ok(3)));

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::Ready(())));
}

#[test]
fn clone_subscribers_after_reading() {
    let mut publisher   = Publisher::new(10);
    let subscriber1     = publisher.subscribe();

    let mut publisher   = executor::spawn(publisher);
    let mut subscriber1 = executor::spawn(subscriber1);

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::Ready(())));

    publisher.wait_send(1).unwrap();
    publisher.wait_send(2).unwrap();
    publisher.wait_send(3).unwrap();

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::NotReady));

    assert!(subscriber1.wait_stream() == Some(Ok(1)));
    assert!(subscriber1.wait_stream() == Some(Ok(2)));

    let mut subscriber2 = executor::spawn(subscriber1.get_ref().clone());
    assert!(subscriber1.wait_stream() == Some(Ok(3)));

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::NotReady));

    assert!(subscriber2.wait_stream() == Some(Ok(3)));

    assert!(publisher.poll_flush_notify(&NotifyHandle::from(&NotifyNothing), 1) == Ok(Async::Ready(())));
}

#[test]
fn complete_on_multiple_subscribers() {
    let mut publisher   = Publisher::new(10);
    let subscriber1     = publisher.subscribe();
    let subscriber2     = publisher.subscribe();

    let mut publisher   = executor::spawn(publisher);
    let mut subscriber1 = executor::spawn(subscriber1);
    let mut subscriber2 = executor::spawn(subscriber2);

    publisher.wait_send(1).unwrap();
    publisher.wait_send(2).unwrap();
    publisher.wait_send(3).unwrap();

    assert!(subscriber1.wait_stream() == Some(Ok(1)));
    assert!(subscriber1.wait_stream() == Some(Ok(2)));
    assert!(subscriber1.wait_stream() == Some(Ok(3)));

    assert!(subscriber2.wait_stream() == Some(Ok(1)));
    assert!(subscriber2.wait_stream() == Some(Ok(2)));
    assert!(subscriber2.wait_stream() == Some(Ok(3)));
}

#[test]
fn skip_messages_sent_before_subscription() {
    let publisher       = Publisher::new(10);
    let mut publisher   = executor::spawn(publisher);

    publisher.wait_send(1).unwrap();
    let subscriber1     = publisher.subscribe();
    publisher.wait_send(2).unwrap();
    let subscriber2     = publisher.subscribe();
    publisher.wait_send(3).unwrap();

    let mut subscriber1 = executor::spawn(subscriber1);
    let mut subscriber2 = executor::spawn(subscriber2);

    assert!(subscriber1.wait_stream() == Some(Ok(2)));
    assert!(subscriber1.wait_stream() == Some(Ok(3)));

    assert!(subscriber2.wait_stream() == Some(Ok(3)));
}

#[test]
fn blocks_if_subscribers_are_full() {
    let mut publisher   = Publisher::new(10);
    let subscriber      = publisher.subscribe();

    let mut publisher   = executor::spawn(publisher);
    let mut subscriber  = executor::spawn(subscriber);

    let mut send_count  = 0;
    while send_count < 100 {
        match publisher.start_send_notify(send_count, &NotifyHandle::from(&NotifyNothing), 0) {
            Ok(AsyncSink::NotReady(_)) => {
                break;
            },

            _ => { }
        }

        send_count += 1;
    }

    assert!(send_count == 10);

    for a in 0..10 {
        assert!(subscriber.wait_stream() == Some(Ok(a)));
    }
}

#[test]
fn single_receive_on_one_subscriber() {
    let mut publisher   = SinglePublisher::new(10);
    let subscriber      = publisher.subscribe();

    let mut publisher   = executor::spawn(publisher);
    let mut subscriber  = executor::spawn(subscriber);

    publisher.wait_send(1).unwrap();
    publisher.wait_send(2).unwrap();
    publisher.wait_send(3).unwrap();

    assert!(subscriber.wait_stream() == Some(Ok(1)));
    assert!(subscriber.wait_stream() == Some(Ok(2)));
    assert!(subscriber.wait_stream() == Some(Ok(3)));
}

#[test]
fn single_receive_on_two_subscribers() {
    let (result_tx, result_rx) = channel();

    let mut publisher   = SinglePublisher::new(1);
    let subscriber1     = publisher.subscribe();
    let subscriber2     = publisher.subscribe();

    let mut publisher   = executor::spawn(publisher);

    // Subscribers will block us if there's nothing listening, so spawn two threads to receive data
    let result = result_tx.clone();
    thread::spawn(move || {
        let mut subscriber1 = executor::spawn(subscriber1);
        result.send(subscriber1.wait_stream()).unwrap();
    });

    let result = result_tx.clone();
    thread::spawn(move || {
        let mut subscriber2 = executor::spawn(subscriber2);
        result.send(subscriber2.wait_stream()).unwrap();
    });

    publisher.wait_send(1).unwrap();
    publisher.wait_send(2).unwrap();

    // Read the results from the threads (they're unordered, but as each thread only reads a single value we'll see if it gets sent to multiple places)
    let msg1 = result_rx.recv().unwrap();
    let msg2 = result_rx.recv().unwrap();

    // Should be 1, 2 in either order
    assert!(msg1 == Some(Ok(1)) || msg1 == Some(Ok(2)));

    if msg1 == Some(Ok(1)) {
        assert!(msg2 == Some(Ok(2)));
    } else {
        assert!(msg2 == Some(Ok(1)));
    }
}

#[test]
fn expiring_removes_oldest_events_first() {
    let mut publisher   = ExpiringPublisher::new(5);
    let subscriber      = publisher.subscribe();

    let mut publisher   = executor::spawn(publisher);
    let mut subscriber  = executor::spawn(subscriber);

    for evt in 0..100 {
        publisher.wait_send(evt).unwrap();
    }

    assert!(subscriber.wait_stream() == Some(Ok(95)));
    assert!(subscriber.wait_stream() == Some(Ok(96)));
    assert!(subscriber.wait_stream() == Some(Ok(97)));
    assert!(subscriber.wait_stream() == Some(Ok(98)));
    assert!(subscriber.wait_stream() == Some(Ok(99)));
}
*/
