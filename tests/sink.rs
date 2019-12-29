extern crate futures;
extern crate flo_stream;

use flo_stream::*;

use futures::prelude::*;
use futures::executor;

use std::thread;

#[test]
pub fn send_via_sink() {
    executor::block_on(async {
        let publisher       = Publisher::new(10);
        let mut publisher   = publisher.to_sink();

        let mut subscriber  = publisher.subscribe();

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
pub fn close_sink() {
    executor::block_on(async {
        let publisher       = Publisher::new(10);
        let mut publisher   = publisher.to_sink();

        let mut subscriber  = publisher.subscribe();

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

