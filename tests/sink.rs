extern crate futures;
extern crate flo_stream;

use flo_stream::*;

use futures::prelude::*;
use futures::executor;
use futures::stream;

use std::thread;

#[test]
pub fn send_via_sink() {
    executor::block_on(async {
        let publisher       = Publisher::new(10);
        let mut publisher   = publisher.to_sink();

        let mut subscriber  = publisher.subscribe().unwrap();

        // .send() flushes the value that's sent into the sink (ie, waits for all of the subscribers to receive it)
        thread::spawn(move || {
            executor::block_on(async {
                publisher.send(1).await.unwrap();
                publisher.send(2).await.unwrap();
                publisher.send(3).await.unwrap();
            });
        });

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));
    })
}

#[test]
pub fn send_stream_via_sink() {
    executor::block_on(async {
        let publisher       = Publisher::new(10);
        let mut publisher   = publisher.to_sink();

        let mut subscriber  = publisher.subscribe().unwrap();

        // .send() flushes the value that's sent into the sink (ie, waits for all of the subscribers to receive it)
        thread::spawn(move || {
            executor::block_on(async {
                publisher.send_all(&mut stream::iter(vec![Ok(1), Ok(2), Ok(3)])).await.unwrap();
            });
        });

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));
    })
}

#[test]
pub fn close_sink() {
    executor::block_on(async {
        let publisher       = Publisher::new(10);
        let mut publisher   = publisher.to_sink();

        let mut subscriber  = publisher.subscribe().unwrap();

        // .send() flushes the value that's sent into the sink (ie, waits for all of the subscribers to receive it)
        thread::spawn(move || {
            executor::block_on(async {
                publisher.send(1).await.unwrap();
                publisher.send(2).await.unwrap();
                publisher.send(3).await.unwrap();

                publisher.close().await.unwrap();
            });
        });

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));

        assert!(subscriber.next().await == None);
    })
}

#[test]
pub fn close_sink_same_thread() {
    executor::block_on(async {
        let publisher       = Publisher::<i32>::new(10);
        let mut publisher   = publisher.to_sink();

        let mut subscriber  = publisher.subscribe().unwrap();

        // Need to use publish() (or our own sink futures) to buffer multiple items as send() always flushes 
        publisher.publish(1).unwrap().await;
        publisher.publish(2).unwrap().await;
        publisher.publish(3).unwrap().await;

        assert!(subscriber.next().await == Some(1));
        assert!(subscriber.next().await == Some(2));
        assert!(subscriber.next().await == Some(3));

        // It's possible to keep the publisher around after closing it
        publisher.close().await.unwrap();

        // Closing the sink should close the subscriber queue
        assert!(subscriber.next().await == None);
    })
}


