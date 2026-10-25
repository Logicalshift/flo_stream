```toml
flo_stream = "0.8"
```

# flo_stream

`flo_stream` is a crate providing some new asynchronous streams for Rust. Its main features are:

* PubSub, or multi-producer, multi-consumer streams: send a message to multiple receivers from multiple producers, with
  some variants for sending to all subscribers or in a round-robin fashion.
* Generator streams, a more ergonomic alternative to `unfold` where the state is too complicated to pass from call to call.
* Synchronous/asynchronous MPSC channels, a way to send and receive data to a stream that can be used either from a
  blocking thread or a non-blocking asynchronous future.

## PubSub

The sink type provided is `Publisher`. You can create one with `let publisher = Publisher::new(10)`. This implements 
the `Sink` trait so can be used in a very similar way to send messages. The number passed in is the maximum number
of waiting messages allowed for any given subscriber.

A subscription can be created using `let subscription = publisher.subscribe()`. Any messages sent to the sink after
this is called is relayed to all subscriptions. A subscription is a `Stream` so can interact with other parts of the
futures library in the usual way.

Here's a full worked example with a single subscriber:

```Rust
let mut publisher       = Publisher::new(10);
let mut subscriber      = publisher.subscribe();

executor::block_on(async {
    publisher.publish(1).await;
    publisher.publish(2).await;
    publisher.publish(3).await;

    assert!(subscriber.next().await == Some(1));
    assert!(subscriber.next().await == Some(2));
    assert!(subscriber.next().await == Some(3));
});
```

It's also possible to call `subscriber.clone()` to create a new subscription from an existing one without needing to 
keep a reference to the publisher. This can be used to reduce the amount of effort needed in passing objects around, and
to hide implementation details from the caller.
