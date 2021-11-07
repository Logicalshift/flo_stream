use futures::prelude::*;
use futures::task::{Waker, Poll, Context};
use futures::stream::{BoxStream};

use std::pin::*;
use std::sync::*;

///
/// The core of a switching stream
///
struct SwitchingStreamCore<TValue> {
    /// The stream to read from
    stream: BoxStream<'static, TValue>,

    /// Set to true if the stream has been closed
    closed: bool,

    /// Set to false once the switch has been removed
    switch_available: bool,

    /// The current waker for the stream
    wake: Option<Waker>
}

///
/// A stream that relays the results from a source stream, which can be switched out if necessary
///
/// This is useful when using a stream as a source of events (eg, as part of a UI application). This can be used to change the source
/// of events being consumed by one part of an application from another part without creating a direct dependency between those two parts.
///
/// For example, in an application that implements a set of tools - a paint program for example - this could be used to direct the flow
/// of events to and from the active tool.
///
/// A switching stream will not close until both the stream it contains has finished and the corresponding switch object has been removed
/// from memory.
///
pub struct SwitchingStream<TValue> {
    core: Arc<Mutex<SwitchingStreamCore<TValue>>>
}

///
/// Used to switch the stream for a corresponding `SwitchingStream` item
///
/// This must be dropped before the corresponding `SwitchingStream` can be closed
///
pub struct StreamSwitch<TValue> {
    core: Arc<Mutex<SwitchingStreamCore<TValue>>>
}

impl<TValue> Stream for SwitchingStream<TValue> {
    type Item = TValue;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context) -> Poll<Option<TValue>> {
        // Lock the core
        let mut core = self.core.lock().unwrap();

        // Remember the waker: this is used if StreamSwitch is used to change the stream that this is returning
        core.wake = Some(context.waker().clone());

        if core.closed {
            // Stream has been closed
            if !core.switch_available {
                // The switch has been removed so there will be no more streams sent here
                Poll::Ready(None)
            } else {
                // The stream is closed but the switch is available: the stream 
                Poll::Pending
            }
        } else {
            // Poll the underlying stream
            let result = core.stream.poll_next_unpin(context);

            match result {
                Poll::Ready(None) => {
                    // Core stream has been closed
                    core.closed = true;

                    if !core.switch_available {
                        // The switch object has been freed
                        Poll::Ready(None)
                    } else {
                        // The switch object may restart the core stream
                        Poll::Pending
                    }
                }

                _ => result
            }
        }
    }
}

impl<TValue> StreamSwitch<TValue> {
    ///
    /// Switches the stream being returned by the corresponding `SwitchingStream` to the specified new stream
    ///
    pub fn switch_to_stream<TStream: 'static+Send+Stream<Item=TValue>>(&self, new_stream: TStream) {
        let waker = {
            let mut core    = self.core.lock().unwrap();

            // Un-close the stream and set it to the new stream that was passed in
            core.closed     = false;
            core.stream     = new_stream.boxed();

            // Wake up the core
            core.wake.take()
        };

        waker.map(|waker| waker.wake());
    }
}

impl<TValue> Drop for StreamSwitch<TValue> {
    fn drop(&mut self) {
        let waker = {
            let mut core = self.core.lock().unwrap();

            // The switch for this core is no longer available, so the stream can close
            core.switch_available = false;

            // Wake up the core
            core.wake.take()
        };

        waker.map(|waker| waker.wake());
    }
}

///
/// Returns a switching stream and its switch, set to initially read from the stream that's passed in
///
/// A switching stream can be changed to return results from another stream at any time. It's quite useful for things like streams
/// of events in particular, where something like a user interface might want to change the source of events around.
///
/// The `StreamSwitch` can be used to change where the results for the returned `SwitchingStream` are generated from. The 
/// `SwitchingStream` will only close once both its underlying stream has finished and the switch has been dropped.
///
pub fn switchable_stream<TStream: 'static+Send+Stream>(stream: TStream) -> (SwitchingStream<TStream::Item>, StreamSwitch<TStream::Item>) {
    let core = SwitchingStreamCore {
        stream:             stream.boxed(),
        closed:             false,
        switch_available:   true,
        wake:               None
    };
    let core = Arc::new(Mutex::new(core));

    let stream = SwitchingStream    { core: Arc::clone(&core) };
    let switch = StreamSwitch       { core: core };

    (stream, switch)
}
