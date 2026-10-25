use futures::prelude::*;
use futures::task::{Waker, Context, Poll};

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
/// A slot in the SyncAsyncChannel
///
enum Slot<TMessage> {
    /// Unused slot
    Empty,

    /// Reserved slot (pending being filled)
    Reserved,

    /// Released slot (won't be filled)
    Released,

    /// Slot with a message available
    Full(TMessage),
}

///
/// The core of a sync/async channel
///
struct SyncAsyncCore<TMessage> {
    /// Wakes the receiver, if it's waiting for a message to send
    recv_waker:         Option<SyncAsyncWaker>,

    /// Slots for waiting messages (plus one extra as the 'full marker')
    waiting:            Vec<Slot<TMessage>>,

    /// Index of the next message to deliver
    read_idx:           usize,

    /// Index of the next message to write
    write_idx:          usize,

    /// True if the receiver is closed
    receiver_closed:    bool,

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
    handle:     usize,
    closed:     bool,
    core:       Arc<Mutex<SyncAsyncCore<TMessage>>>,
    write_idx:  Option<usize>,
}

///
/// A MPSC channel receiver that can be used synchronously or asynchronously
///
pub struct RecvSyncAsync<TMessage> {
    core: Arc<Mutex<SyncAsyncCore<TMessage>>>,
}

impl<TMessage> Slot<TMessage> {
    ///
    /// Takes the value from this slot
    ///
    #[inline]
    fn take(&mut self) -> Slot<TMessage> {
        use std::mem;

        match self {
            Slot::Reserved  => mem::replace(self, Slot::Reserved),
            _               => mem::replace(self, Slot::Empty),
        }
    }
}

impl<TMessage> SyncAsyncCore<TMessage> {
    ///
    /// Stores a message in this core (or bounces it back if there's no space)
    ///
    #[inline]
    fn enqueue(&mut self, msg: TMessage) -> Result<(), TMessage> {
        // Get the write index we'll save after writing the current message
        let next_write_idx = self.write_idx + 1;
        let next_write_idx = if next_write_idx >= self.waiting.len() { 0 } else { next_write_idx };

        if next_write_idx == self.read_idx {
            // Next position has overrun
            Err(msg)
        } else {
            // Store this message and update the write position
            self.waiting[self.write_idx] = Slot::Full(msg);
            self.write_idx = next_write_idx;

            Ok(())
        }
    }

    ///
    /// Returns a write index without actually storing anything
    ///
    fn reserve(&mut self) -> Result<usize, ()> {
        let next_write_idx = self.write_idx + 1;
        let next_write_idx = if next_write_idx >= self.waiting.len() { 0 } else { next_write_idx };

        if next_write_idx == self.read_idx {
            // Next position has overrun
            Err(())
        } else {
            // Store this message and update the write position
            let write_idx = self.write_idx;
            self.write_idx = next_write_idx;

            self.waiting[write_idx] = Slot::Reserved;

            Ok(write_idx)
        }
    }

    ///
    /// Dequeues a message from this core if there's one waiting
    ///
    #[inline]
    fn dequeue(&mut self) -> Result<TMessage, ()> {
        loop {
            match self.waiting[self.read_idx].take() {
                Slot::Full(msg) => {
                    // Value waiting in the queue
                    let next_read_idx = self.read_idx + 1;
                    let next_read_idx = if next_read_idx >= self.waiting.len() { 0 } else { next_read_idx };

                    self.read_idx = next_read_idx;

                    return Ok(msg)
                }

                Slot::Reserved => { 
                    // Slot waiting to be filled
                    return Err(()) 
                }

                Slot::Empty => {
                    // Slot not in use
                    return Err(())
                }

                Slot::Released => {
                    // Slot was reserved but never received a value: skip and try again
                    let next_read_idx = self.read_idx + 1;
                    let next_read_idx = if next_read_idx >= self.waiting.len() { 0 } else { next_read_idx };

                    self.read_idx = next_read_idx;
                }
            }
        }
    }

    ///
    /// True if there are no waiting messages
    ///
    #[inline]
    fn is_empty(&self) -> bool {
        self.write_idx == self.read_idx
    }

    ///
    /// Removes a reserved write position from this channel
    ///
    fn unreserve(&mut self, idx: usize) {
        // Mark the slot as released, gets tidied up next receive
        self.waiting[idx] = Slot::Released;
    }
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
        if core.max_waiting == 0 && core.waiting.len() == 1 {
            // 'Block until ready to receive' mode: unblock any waiting receivers
            core.waiting.push(Slot::Empty);
            let wakers = core.waiting_senders.drain(..).collect::<Vec<_>>();
            drop(core);

            for (_, waker) in wakers { waker.wake() }

            core = self.core.lock().unwrap();
        }

        if let Ok(next) = core.dequeue() {
            // There's a value waiting
            let mut wakers = core.waiting_senders.drain(..).collect::<Vec<_>>();

            if core.is_empty() {
                // Nothing waiting to be sent: wake up anything waiting on a flush
                wakers.extend(core.waiting_flushers.drain(..));
            }

            // If we're in 'block until ready to receive' mode, shrink the waiting list
            if core.max_waiting == 0 && core.waiting.len() > 1 {
                core.waiting.pop();
                core.write_idx = 0;
                core.read_idx  = 0;
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

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.closed { return Poll::Ready(Err(())); }

        self.unreserve();
        let mut core = self.core.lock().unwrap();

        if core.receiver_closed {
            // Nothing receiving
            Poll::Ready(Err(()))
        } else if let Ok(write_idx) = core.reserve() {
            drop(core);
            self.write_idx = Some(write_idx);

            Poll::Ready(Ok(()))
        } else {
            // Too many things queued: set a waker
            if let Some((_, waker)) = core.waiting_senders.iter_mut().filter(|(handle, _)| *handle == self.handle).next() {
                *waker = SyncAsyncWaker::Asynchronous(cx.waker().clone());
            } else {
                core.waiting_senders.push((self.handle, SyncAsyncWaker::Asynchronous(cx.waker().clone())));
            }

            Poll::Pending
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: TMessage) -> Result<(), Self::Error> {
        let Some(write_idx) = self.write_idx.take() else { return Err(()) };

        // Store this item in the core
        let mut core = self.core.lock().unwrap();
        core.waiting[write_idx] = Slot::Full(item);

        // Wake up any receivers
        let waker = core.recv_waker.take();
        drop(core);

        if let Some(waker) = waker { waker.wake(); };

        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.closed { return Poll::Ready(Err(())); }

        self.unreserve();

        let mut core = self.core.lock().unwrap();

        if core.is_empty() {
            // No more items waiting in the core
            Poll::Ready(Ok(()))
        } else if core.receiver_closed {
            // Nothing receiving, but things waiting
            Poll::Ready(Err(()))
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
        if self.closed { return Poll::Ready(Ok(())); }

        self.unreserve();

        // Poll for the core to be flushed
        let mut core = self.core.lock().unwrap();

        if core.is_empty() {
            // No more items waiting in the core: count this sender as closed
            core.closed_count += 1;

            let waker = core.recv_waker.take();
            drop(core);

            // Wake anything waiting to receive things
            if let Some(waker) = waker { waker.wake(); }

            self.closed = true;

            Poll::Ready(Ok(()))
        } else if core.receiver_closed {
            // Nothing receving, messages waiting
            Poll::Ready(Err(()))
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

            if core.max_waiting == 0 && core.waiting.len() == 1 {
                // 'Wait until ready to receive' mode: unblock the waiting list
                core.waiting.push(Slot::Empty);
                let wakers = core.waiting_senders.drain(..).collect::<Vec<_>>();
                drop(core);

                for (_, waker) in wakers { waker.wake() }

                core = self.core.lock().unwrap();
            }

            core.recv_waker = None;

            if let Ok(next) = core.dequeue() {
                // Deliver this message
                let mut wakers = core.waiting_senders.drain(..).collect::<Vec<_>>();

                if core.max_waiting == 0 && core.waiting.len() > 1 {
                    // 'Wait until ready to receive' mode: block the waiting list
                    core.waiting.pop();
                    core.write_idx = 0;
                    core.read_idx  = 0;
                }

                if core.is_empty() {
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
    /// If this has reserved a write index, then unreserve it
    ///
    fn unreserve(&mut self) {
        if let Some(write_idx) = self.write_idx.take() {
            let mut core = self.core.lock().unwrap();

            // Unreserve the write index
            core.unreserve(write_idx);

            // Wake up anything that might want the freed slot
            let mut wakers = core.waiting_senders.drain(..).map(|(_, waker)| waker).collect::<Vec<_>>();
            wakers.extend(core.recv_waker.take());
            if core.is_empty() {
                wakers.extend(core.waiting_flushers.drain(..).map(|(_, waker)| waker));
            }

            drop(core);

            for waker in wakers { waker.wake(); }
        }
    }

    ///
    /// Sends a message to the queue (blocking if the queue is full)
    ///
    pub fn send_sync(&mut self, message: TMessage) -> Result<(), ()> {
        if self.closed {
            // Can't send on a closed stream
            return Err(())
        }

        self.unreserve();

        let mut message = message;
        loop {
            let mut core = self.core.lock().unwrap();

            if core.receiver_closed {
                // Receiver no longer receiving
                return Err(())
            } else if let Err(bounced_message) = core.enqueue(message) {
                message = bounced_message;

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
                let waker = core.recv_waker.take();
                drop(core);

                // Wake up the receiver
                if let Some(waker) = waker { waker.wake(); }

                return Ok(());
            }
        }
    }
}

impl<TMessage> Drop for RecvSyncAsync<TMessage> {
    fn drop(&mut self) {
        let wakers = {
            let mut core = self.core.lock().unwrap();
            core.receiver_closed = true;

            let mut wakers = core.waiting_senders.drain(..).collect::<Vec<_>>();
            wakers.extend(core.waiting_flushers.drain(..));

            wakers
        };

        for (_, waker) in wakers { waker.wake() }
    }
}

impl<TMessage> Drop for SendSyncAsync<TMessage> {
    fn drop(&mut self) {
        self.unreserve();

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

impl<TMessage> Clone for SendSyncAsync<TMessage> {
    fn clone(&self) -> Self {
        // Assign a new handle (keep closed if closed)
        let mut core    = self.core.lock().unwrap();
        let handle      = core.next_sender_handle;

        core.next_sender_handle += 1;
        if self.closed {
            core.closed_count += 1;
        }

        drop(core);

        // New sender
        Self {
            handle:     handle, 
            closed:     self.closed, 
            core:       self.core.clone(),
            write_idx:  None,
        }
    }
}

///
/// Creates a MPSC channel that can be used either synchronously or asynchronously
///
/// The queue length is the number of messages this will queue before blocking (0 creates
/// a queue that will block until the message is ready to be received).
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
    let mut waiting = Vec::with_capacity(queue_len+1);
    while waiting.len() <= queue_len { waiting.push(Slot::Empty); }

    let core = SyncAsyncCore {
        recv_waker:         None,
        waiting:            waiting,
        read_idx:           0,
        write_idx:          0,
        max_waiting:        queue_len,
        next_sender_handle: 1,
        closed_count:       0,
        waiting_senders:    vec![],
        waiting_flushers:   vec![],
        receiver_closed:    false,
    };
    let core = Arc::new(Mutex::new(core));

    // Default sender gets handle 0
    let sender = SendSyncAsync {
        handle:     0,
        closed:     false,
        write_idx:  None,
        core:       core.clone(),
    };
    let receiver = RecvSyncAsync {
        core: core.clone()
    };

    (sender, receiver)
}
