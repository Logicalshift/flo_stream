use flo_stream::*;

use futures::prelude::*;
use futures::executor;

use std::thread;

#[test]
fn send_and_receive_synchronously() {
    let (send, recv) = channel_sync_or_async(0);

    thread::spawn(move || {
        let mut send = send;
        send.send_sync(42).unwrap();
    });

    let mut recv = recv;
    let received = recv.next_sync();

    assert!(received == Some(42), "{:?}", received);
}

#[test]
fn send_and_receive_asynchronously() {
    let (send, recv) = channel_sync_or_async(0);

    thread::spawn(move || {
        let mut send = send;
        executor::block_on(async move {
            send.send(42).await.unwrap();
        });
    });

    let mut recv = recv;
    let received = executor::block_on(async move { recv.next().await });

    assert!(received == Some(42), "{:?}", received);
}
