//!
//! `flo_stream` is a crate providing some extra utilities for streams in Rust's `futures` library, in particular the 'pubsub' pattern.
//! 
//! The primary new feature is a "pubsub" mechanism - a way to subscribe to updates sent to a futures `Sink`. This differs 
//! from the `Sender`/`Receiver` mechanism provided in the main futures library in two key ways: it's possible to have 
//! multiple receivers, and messages sent when there is no subscriber connected will be ignored.
//! 
//! ## PubSub
//! 
//! The sink type provided is `Publisher`. You can create one with `let publisher = Publisher::new(10)`. This implements 
//! the `Sink` trait so can be used in a very similar way to send messages. The number passed in is the maximum number
//! of waiting messages allowed for any given subscriber.
//! 
//! A subscription can be created using `let subscription = publisher.subscribe()`. Any messages sent to the sink after
//! this is called is relayed to all subscriptions. A subscription is a `Stream` so can interact with other parts of the
//! futures library in the usual way.
//! 
//! Here's a full worked example with a single subscriber.
//! 
//! ```
//! # extern crate flo_stream;
//! # extern crate futures;
//! # use flo_stream::*;
//! # use futures::prelude::*;
//! # use futures::executor;
//! let mut publisher       = Publisher::new(10);
//! let mut subscriber      = publisher.subscribe();
//! 
//! executor::block_on(async {
//!     publisher.publish(1).await;
//!     publisher.publish(2).await;
//!     publisher.publish(3).await;
//! 
//!     assert!(subscriber.next().await == Some(1));
//!     assert!(subscriber.next().await == Some(2));
//!     assert!(subscriber.next().await == Some(3));
//! });
//! ```

#![warn(bare_trait_objects)]

extern crate futures;

mod message_publisher;
mod publisher;
mod sink;
mod blocking_publisher;
mod single_publisher;
mod expiring_publisher;
mod subscriber;
mod pubsub_core;
mod spawn;

pub use self::message_publisher::*;
pub use self::publisher::*;
pub use self::sink::*;
pub use self::expiring_publisher::*;
pub use self::blocking_publisher::*;
pub use self::single_publisher::*;
pub use self::subscriber::*;
pub use self::spawn::*;
