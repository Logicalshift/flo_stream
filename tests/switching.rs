use flo_stream::*;

use futures::prelude::*;
use futures::stream;
use futures::executor;

use ::desync::*;

use std::mem;
use std::thread;
use std::time::{Duration};

#[test]
fn switch_stream() {
    // Two streams to switch between
    let stream_1            = stream::iter(vec![1, 2, 3, 4]);
    let stream_2            = stream::iter(vec![10, 11, 12, 13]);

    // Create a switchable stream reading from stream_1 initially
    let (stream, switch)    = switchable_stream(stream_1);
    let mut stream          = stream;

    // Read the first two values from stream_1
    let a = executor::block_on(async { stream.next().await.unwrap() });
    let b = executor::block_on(async { stream.next().await.unwrap() });

    // Switch to 2
    switch.switch_to_stream(stream_2);

    // Last three values from stream_2
    let c = executor::block_on(async { stream.next().await.unwrap() });
    let d = executor::block_on(async { stream.next().await.unwrap() });
    let e = executor::block_on(async { stream.next().await.unwrap() });

    // a and b are read from stream 1
    assert!(a == 1);
    assert!(b == 2);

    // c, d and e are read from stream 2
    assert!(c == 10);
    assert!(d == 11);
    assert!(e == 12);
}

#[test]
fn close_when_switch_is_dropped() {
    // Create a stream
    let stream_1            = stream::iter(vec![1, 2, 3, 4]);

    // Create a switchable stream reading from stream_1 initially
    let (stream, switch)    = switchable_stream(stream_1);
    let mut stream          = stream;

    // Drop the switch so the switchable stream can close
    mem::drop(switch);

    // Read the 4 values from stream_1
    executor::block_on(async { stream.next().await.unwrap() });
    executor::block_on(async { stream.next().await.unwrap() });
    executor::block_on(async { stream.next().await.unwrap() });
    executor::block_on(async { stream.next().await.unwrap() });

    // Should now be closed
    let closed = executor::block_on(async { stream.next().await });

    assert!(closed.is_none());
}

#[test]
fn switch_after_first_stream_is_closed() {
    // Thing to run tasks in the background
    let background          = Desync::new(());

    // Two streams to switch between
    let stream_1            = stream::iter(vec![1, 2, 3, 4]);
    let stream_2            = stream::iter(vec![10, 11, 12, 13]);

    // Create a switchable stream reading from stream_1 initially
    let (stream, switch)    = switchable_stream(stream_1);
    let mut stream          = stream;

    // Read all of the values from stream_1
    let a = executor::block_on(async { stream.next().await.unwrap() });
    let b = executor::block_on(async { stream.next().await.unwrap() });
    let c = executor::block_on(async { stream.next().await.unwrap() });
    let d = executor::block_on(async { stream.next().await.unwrap() });

    // Start reading the next value
    let next_value = background.future_desync(move |_| async move { stream.next().await }.boxed());

    // Wait for a bit to make sure the future is running
    thread::sleep(Duration::from_millis(100));

    // Switch to 2
    switch.switch_to_stream(stream_2);

    // Should read the first few values
    assert!(a == 1);
    assert!(b == 2);
    assert!(c == 3);
    assert!(d == 4);

    // Should now read the first value from stream_2
    let e = executor::block_on(async { next_value.await.unwrap() });
    assert!(e == Some(10));
}
