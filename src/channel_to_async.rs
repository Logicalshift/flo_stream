use futures::prelude::*;
use futures::task::{Waker, Context, Poll};

use std::collections::*;
use std::thread;
use std::thread::{Thread};
use std::sync::*;
use std::pin::*;

///
/// A waker for a SyncAsync channel
///
enum SyncAsyncWaker {
    Asynchronous(Waker),
    Synchronous(Thread),
}

///
/// The core of a sync/async channel
///
struct SyncAsyncCore<TMessage> {
    /// Used to wake this core when the thread is being waited on multiple times
    recv_waker:         Option<SyncAsyncWaker>,

    /// Undelivered messages that are waiting
    waiting:            VecDeque<TMessage>,

    /// Maximum number of messages to enqueue before blocking
    max_waiting:        usize,

    /// Handle to assign to the next sender that we create
    next_sender_handle: usize,

    /// Number of senders that have been dropped (= next_send_handle if there are no senders left)
    closed_count:       usize,

    /// Senders that are blocked waiting for the queue size to go down
    waiting_senders:    Vec<(usize, SyncAsyncWaker)>,

    /// Things that are waiting for the 
    waiting_flushers:   Vec<(usize, SyncAsyncWaker)>,
}

///
/// A MPSC channel sender that can be used synchronously or asynchronously
///
pub struct SendSyncAsync<TMessage> {
    handle: usize,
    closed: bool,
    core:   Arc<Mutex<SyncAsyncCore<TMessage>>>,
}

///
/// A MPSC channel receiver that can be used synchronously or asynchronously
///
pub struct RecvSyncAsync<TMessage> {
    core: Arc<Mutex<SyncAsyncCore<TMessage>>>,
}

impl SyncAsyncWaker {
    ///
    /// Wake whatever is waiting for this waker
    ///
    pub fn wake(self) {
        match self {
            SyncAsyncWaker::Asynchronous(waker) => { waker.wake(); },
            SyncAsyncWaker::Synchronous(thread) => { thread.unpark(); },
        }
    }
}

impl<TMessage> Stream for RecvSyncAsync<TMessage> {
    type Item = TMessage;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut core = self.core.lock().unwrap();

        core.recv_waker = None;

        if let Some(next) = core.waiting.pop_front() {
            // There's a value waiting
            let mut wakers = core.waiting_senders.drain(..).collect::<Vec<_>>();

            if core.waiting.is_empty() {
                // Nothing waiting to be sent: wake up anything waiting on a flush
                wakers.extend(core.waiting_flushers.drain(..));
            }

            // Wake up anything waiting for a slot or for the stream to flush
            drop(core);
            for (_, waker) in wakers { waker.wake() };

            Poll::Ready(Some(next))
        } else if core.next_sender_handle == core.closed_count {
            // All the senders are closed/dropped
            Poll::Ready(None)
        } else {
            // Retry when we're woken up
            core.recv_waker = Some(SyncAsyncWaker::Asynchronous(cx.waker().clone()));
            Poll::Pending
        }
    }
}

impl<TMessage> Sink<TMessage> for SendSyncAsync<TMessage> {
    type Error = ();

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.closed { return Poll::Ready(Err(())); }

        let mut core = self.core.lock().unwrap();

        if core.waiting.len() > core.max_waiting {
            // Too many things queued: set a waker
            if let Some((_, waker)) = core.waiting_senders.iter_mut().filter(|(handle, _)| *handle == self.handle).next() {
                *waker = SyncAsyncWaker::Asynchronous(cx.waker().clone());
            } else {
                core.waiting_senders.push((self.handle, SyncAsyncWaker::Asynchronous(cx.waker().clone())));
            }

            Poll::Pending
        } else {
            // There's a slot available
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(self: Pin<&mut Self>, item: TMessage) -> Result<(), Self::Error> {
        // TODO: have to mess around with reserved slots to avoid a race condition that can overfill the queue

        // Store this item in the core
        let mut core = self.core.lock().unwrap();
        core.waiting.push_back(item);

        // Wake up any receivers
        let waker = core.recv_waker.take();
        drop(core);

        if let Some(waker) = waker { waker.wake(); };

        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.closed { return Poll::Ready(Err(())); }

        let mut core = self.core.lock().unwrap();

        if core.waiting.is_empty() {
            // No more items waiting in the core
            Poll::Ready(Ok(()))
        } else {
            // Wait for the recv stream to signal that all the messages have been delivered
            if let Some((_, waker)) = core.waiting_flushers.iter_mut().filter(|(handle, _)| *handle == self.handle).next() {
                *waker = SyncAsyncWaker::Asynchronous(cx.waker().clone())
            } else {
                core.waiting_flushers.push((self.handle, SyncAsyncWaker::Asynchronous(cx.waker().clone())))
            }

            Poll::Pending
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.closed { return Poll::Ready(Err(())); }

        // Poll for the core to be flushed
        let mut core = self.core.lock().unwrap();

        if core.waiting.is_empty() {
            // No more items waiting in the core: count this sender as closed
            core.closed_count += 1;
            drop(core);

            self.closed = true;

            Poll::Ready(Ok(()))
        } else {
            // Wait for the recv stream to signal that all the messages have been delivered
            if let Some((_, waker)) = core.waiting_flushers.iter_mut().filter(|(handle, _)| *handle == self.handle).next() {
                *waker = SyncAsyncWaker::Asynchronous(cx.waker().clone())
            } else {
                core.waiting_flushers.push((self.handle, SyncAsyncWaker::Asynchronous(cx.waker().clone())))
            }

            Poll::Pending
        }
    }
}

impl<TMessage> RecvSyncAsync<TMessage> {
    ///
    /// Reads the next message from this queue (blocking until one is sent)
    ///
    pub fn next_sync(&mut self) -> Option<TMessage> {
        loop {
            let mut core = self.core.lock().unwrap();

            if let Some(next) = core.waiting.pop_front() {
                // Deliver this message
                let mut wakers = core.waiting_senders.drain(..).collect::<Vec<_>>();

                if core.waiting.is_empty() {
                    wakers.extend(core.waiting_flushers.drain(..));
                }
                drop(core);

                // Wake anything waiting for the waiting state to change
                for (_, waker) in wakers { waker.wake(); }

                return Some(next);
            } else if core.closed_count == core.next_sender_handle {
                // All senders are closed
                return None;
            } else {
                // Block until something sends a message on a different thread
                core.recv_waker = Some(SyncAsyncWaker::Synchronous(thread::current()));

                drop(core);
                thread::park();
            }
        }
    }
}

impl<TMessage> SendSyncAsync<TMessage> {
    ///
    /// Reads the next message from this queue (blocking until one is sent)
    ///
    pub fn send_sync(&mut self, message: TMessage) {
        loop {
            let mut core = self.core.lock().unwrap();
            core.recv_waker = None;

            if core.waiting.len() > core.max_waiting {
                // Block until there's a free slot
                if let Some((_, waker)) = core.waiting_senders.iter_mut().filter(|(handle, _)| *handle == self.handle).next() {
                    *waker = SyncAsyncWaker::Synchronous(thread::current())
                } else {
                    core.waiting_senders.push((self.handle, SyncAsyncWaker::Synchronous(thread::current())));
                }
                drop(core);

                thread::park();
            } else {
                // Send this message
                core.waiting.push_back(message);
                let waker = core.recv_waker.take();
                drop(core);

                // Wake up the receiver
                if let Some(waker) = waker { waker.wake(); }

                break;
            }
        }
    }
}

impl<TMessage> Drop for SendSyncAsync<TMessage> {
    fn drop(&mut self) {
        // Close off this sender
        let waker = if !self.closed {
            let mut core = self.core.lock().unwrap();
            core.closed_count += 1;
            core.recv_waker.take()
        } else {
            None
        };

        if let Some(waker) = waker { waker.wake() }
    }
}

///
/// Creates a MPSC channel that can be used either synchronously or asynchronously
///
/// This is useful with code that needs to interoperate between standard synchronous
/// threads and futures. When in synchronous mode, backpressure is supplied by parking the
/// thread and blocking. In asynchronous mode, backpressure is via the context's waker
/// mechanism. Either the sender or the receiver can be used in either mode.
///
/// This pairs well with the `desync` crate, which can schedule a mix of synchronous and
/// asynchronous operations, or it can be used 'raw' when launching a thread to carry out
/// a synchronous operation from an async thread.
///
pub fn channel_sync_or_async<TMessage>(queue_len: usize) -> (SendSyncAsync<TMessage>, RecvSyncAsync<TMessage>) {
    // Create an empty core
    let core = SyncAsyncCore {
        recv_waker:         None,
        waiting:            VecDeque::new(),
        max_waiting:        queue_len,
        next_sender_handle: 1,
        closed_count:       0,
        waiting_senders:    vec![],
        waiting_flushers:   vec![],
    };
    let core = Arc::new(Mutex::new(core));

    // Default sender gets handle 0
    let sender = SendSyncAsync {
        handle: 0,
        closed: false,
        core:   core.clone()
    };
    let receiver = RecvSyncAsync {
        core: core.clone()
    };

    (sender, receiver)
}
