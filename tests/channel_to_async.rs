use flo_stream::*;

use futures::prelude::*;
use futures::executor;

use std::thread;
use std::pin::*;
use std::sync::mpsc;
use std::time::{Duration};

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

#[test]
fn send_sync_receive_async() {
    let (send, recv) = channel_sync_or_async(0);

    thread::spawn(move || {
        let mut send = send;
        send.send_sync(42).unwrap();
    });

    let mut recv = recv;
    let received = executor::block_on(async move { recv.next().await });

    assert!(received == Some(42), "{:?}", received);
}

#[test]
fn send_async_receive_sync() {
    let (send, recv) = channel_sync_or_async(0);

    thread::spawn(move || {
        let mut send = send;
        executor::block_on(async move {
            send.send(42).await.unwrap();
        });
    });

    let mut recv = recv;
    let received = recv.next_sync();

    assert!(received == Some(42), "{:?}", received);
}

///
/// Runs a test on a separate thread, failing if it takes too long (so deadlocks fail rather than hang)
///
fn with_timeout<TFn: 'static + Send + FnOnce()>(test: TFn) {
    let (done_send, done_recv) = mpsc::channel();

    thread::spawn(move || {
        test();
        done_send.send(()).ok();
    });

    done_recv.recv_timeout(Duration::from_secs(5)).expect("Test timed out or panicked");
}

#[test]
fn send_many_through_buffer_synchronously() {
    with_timeout(|| {
        let (mut send, mut recv) = channel_sync_or_async(4);

        let sender = thread::spawn(move || {
            for idx in 0..1000 { send.send_sync(idx).unwrap(); }
        });

        for idx in 0..1000 { assert!(recv.next_sync() == Some(idx)); }

        sender.join().unwrap();
        assert!(recv.next_sync() == None);
    });
}

#[test]
fn zero_length_round_trips_synchronously() {
    with_timeout(|| {
        let (mut send, mut recv) = channel_sync_or_async(0);

        let sender = thread::spawn(move || {
            for idx in 0..200 { send.send_sync(idx).unwrap(); }
        });

        for idx in 0..200 { assert!(recv.next_sync() == Some(idx)); }

        sender.join().unwrap();
        assert!(recv.next_sync() == None);
    });
}

#[test]
fn zero_length_round_trips_asynchronously() {
    with_timeout(|| {
        let (mut send, mut recv) = channel_sync_or_async(0);

        let sender = thread::spawn(move || {
            executor::block_on(async move {
                for idx in 0..200 { send.send(idx).await.unwrap(); }
            });
        });

        executor::block_on(async {
            for idx in 0..200 { assert!(recv.next().await == Some(idx)); }
        });

        sender.join().unwrap();
        assert!(executor::block_on(recv.next()) == None);
    });
}

#[test]
fn zero_length_mixed_sync_and_async_senders() {
    with_timeout(|| {
        let (send, mut recv) = channel_sync_or_async(0);
        let mut senders = vec![];

        for sender_idx in 0..3 {
            let mut send = send.clone();
            senders.push(thread::spawn(move || {
                for idx in 0..300 { send.send_sync(sender_idx*1000 + idx).unwrap(); }
            }));
        }

        for sender_idx in 3..6 {
            let mut send = send.clone();
            senders.push(thread::spawn(move || {
                executor::block_on(async move {
                    for idx in 0..300 { send.send(sender_idx*1000 + idx).await.unwrap(); }
                });
            }));
        }

        drop(send);

        // Messages from each sender should arrive in order
        let mut next_expected   = vec![0; 6];
        let mut count           = 0;
        executor::block_on(async {
            while let Some(msg) = recv.next().await {
                let sender_idx = msg / 1000;
                assert!(msg % 1000 == next_expected[sender_idx], "{:?} {:?}", msg, next_expected);

                next_expected[sender_idx] += 1;
                count += 1;
            }
        });

        assert!(count == 1800, "{:?}", count);
        for sender in senders { sender.join().unwrap(); }
    });
}

#[test]
fn stream_ends_after_all_cloned_senders_dropped() {
    with_timeout(|| {
        let (send, mut recv) = channel_sync_or_async(2);

        let senders = (0..4).map(|sender_idx| {
            let mut send = send.clone();
            thread::spawn(move || {
                for idx in 0..100 { send.send_sync(sender_idx*1000 + idx).unwrap(); }
            })
        }).collect::<Vec<_>>();
        drop(send);

        let mut count = 0;
        while let Some(_) = recv.next_sync() { count += 1; }

        assert!(count == 400, "{:?}", count);
        for sender in senders { sender.join().unwrap(); }
    });
}

#[test]
fn close_after_cancelled_send() {
    with_timeout(|| {
        let (mut send, mut recv) = channel_sync_or_async::<i32>(1);

        let receiver = thread::spawn(move || executor::block_on(recv.next()));
        thread::sleep(Duration::from_millis(50));

        executor::block_on(async {
            // Reserve a slot, then close without ever sending anything
            future::poll_fn(|cx| Pin::new(&mut send).poll_ready(cx)).await.unwrap();
            send.close().await.unwrap();
        });

        assert!(receiver.join().unwrap() == None);
    });
}

#[test]
fn close_after_cancelled_send_single_thread() {
    with_timeout(|| {
        let (mut send, mut recv) = channel_sync_or_async::<i32>(1);

        executor::block_on(async {
            let receive = async { recv.next().await };
            let close   = async {
                future::poll_fn(|cx| Pin::new(&mut send).poll_ready(cx)).await.unwrap();
                send.close().await.unwrap();
            };

            let (received, _) = future::join(receive, close).await;
            assert!(received == None, "{:?}", received);
        });
    });
}

#[test]
fn released_slot_frees_space_for_other_sender() {
    with_timeout(|| {
        let (mut send_a, mut recv) = channel_sync_or_async::<i32>(1);
        let mut send_b = send_a.clone();

        executor::block_on(async {
            // A takes the only slot, so B has to wait until A releases it
            future::poll_fn(|cx| Pin::new(&mut send_a).poll_ready(cx)).await.unwrap();

            let send_b      = async { send_b.send(7).await.unwrap(); };
            let release_a   = async { send_a.flush().await.unwrap(); };
            let receive     = async { recv.next().await };

            let (_, _, received) = future::join3(send_b, release_a, receive).await;
            assert!(received == Some(7), "{:?}", received);
        });
    });
}

#[test]
fn reserve_and_release_repeatedly() {
    with_timeout(|| {
        let (send, mut recv) = channel_sync_or_async(1);

        let senders = (0..4).map(|sender_idx| {
            let mut send = send.clone();
            thread::spawn(move || {
                executor::block_on(async move {
                    for idx in 0..300 {
                        if idx % 3 == 0 {
                            // Reserve a slot and give it up again
                            future::poll_fn(|cx| Pin::new(&mut send).poll_ready(cx)).await.unwrap();
                            send.flush().await.unwrap();
                        }

                        send.send(sender_idx*1000 + idx).await.unwrap();
                    }
                });
            })
        }).collect::<Vec<_>>();
        drop(send);

        let mut count = 0;
        while let Some(_) = recv.next_sync() { count += 1; }

        assert!(count == 1200, "{:?}", count);
        for sender in senders { sender.join().unwrap(); }
    });
}

#[test]
fn poll_ready_twice_reserves_one_slot() {
    with_timeout(|| {
        let (mut send, mut recv) = channel_sync_or_async(2);

        executor::block_on(async {
            future::poll_fn(|cx| Pin::new(&mut send).poll_ready(cx)).await.unwrap();
            future::poll_fn(|cx| Pin::new(&mut send).poll_ready(cx)).await.unwrap();
            Pin::new(&mut send).start_send(1).unwrap();
        });

        // If the second poll_ready reserved another slot, the queue would be full and this would block
        send.send_sync(2).unwrap();

        assert!(recv.next_sync() == Some(1));
        assert!(recv.next_sync() == Some(2));
    });
}

#[test]
fn start_send_without_reservation_is_an_error() {
    let (mut send, _recv) = channel_sync_or_async(1);

    assert!(Pin::new(&mut send).start_send(1).is_err());
}

#[test]
fn dropping_receiver_unblocks_sync_sender() {
    with_timeout(|| {
        let (mut send, recv) = channel_sync_or_async(1);
        send.send_sync(1).unwrap();

        // Queue is full, so this blocks until the receiver goes away
        let sender = thread::spawn(move || send.send_sync(2));
        thread::sleep(Duration::from_millis(50));
        drop(recv);

        assert!(sender.join().unwrap().is_err());
    });
}

#[test]
fn dropping_receiver_errors_async_sender() {
    with_timeout(|| {
        let (mut send, recv) = channel_sync_or_async(1);
        send.send_sync(1).unwrap();

        let sender = thread::spawn(move || executor::block_on(async move { send.send(2).await }));
        thread::sleep(Duration::from_millis(50));
        drop(recv);

        assert!(sender.join().unwrap().is_err());
    });
}
