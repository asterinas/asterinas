// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use aster_rights::{Read, ReadOp, TRights, Write, WriteOp};
use aster_rights_proc::require;
use ringbuf::{HeapConsumer as HeapRbConsumer, HeapProducer as HeapRbProducer, HeapRb};

use super::StatusFlags;
use crate::{
    events::{IoEvents, Observer},
    prelude::*,
    process::signal::{Pollable, Pollee, Poller},
};

/// A unidirectional communication channel, intended to implement IPC, e.g., pipe,
/// unix domain sockets, etc.
pub struct Channel<T> {
    producer: Producer<T>,
    consumer: Consumer<T>,
}

impl<T> Channel<T> {
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        Self::with_capacity_and_flags(capacity, StatusFlags::empty())
    }

    pub fn with_capacity_and_flags(capacity: usize, flags: StatusFlags) -> Result<Self> {
        let common = Arc::new(Common::with_capacity_and_flags(capacity, flags)?);
        let producer = Producer(Fifo::new(common.clone()));
        let consumer = Consumer(Fifo::new(common));
        Ok(Self { producer, consumer })
    }

    pub fn split(self) -> (Producer<T>, Consumer<T>) {
        let Self { producer, consumer } = self;
        (producer, consumer)
    }

    pub fn producer(&self) -> &Producer<T> {
        &self.producer
    }

    pub fn consumer(&self) -> &Consumer<T> {
        &self.consumer
    }

    pub fn capacity(&self) -> usize {
        self.producer.0.common.capacity()
    }
}

pub struct Producer<T>(Fifo<T, WriteOp>);

pub struct Consumer<T>(Fifo<T, ReadOp>);

macro_rules! impl_common_methods_for_channel {
    () => {
        pub fn shutdown(&self) {
            self.this_end().shutdown()
        }

        pub fn is_shutdown(&self) -> bool {
            self.this_end().is_shutdown()
        }

        pub fn is_peer_shutdown(&self) -> bool {
            self.peer_end().is_shutdown()
        }

        pub fn status_flags(&self) -> StatusFlags {
            self.this_end().status_flags()
        }

        pub fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
            self.this_end().set_status_flags(new_flags)
        }

        pub fn is_nonblocking(&self) -> bool {
            self.this_end()
                .status_flags()
                .contains(StatusFlags::O_NONBLOCK)
        }

        pub fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
            self.this_end().pollee.poll(mask, poller)
        }

        pub fn register_observer(
            &self,
            observer: Weak<dyn Observer<IoEvents>>,
            mask: IoEvents,
        ) -> Result<()> {
            self.this_end().pollee.register_observer(observer, mask);
            Ok(())
        }

        pub fn unregister_observer(
            &self,
            observer: &Weak<dyn Observer<IoEvents>>,
        ) -> Option<Weak<dyn Observer<IoEvents>>> {
            self.this_end().pollee.unregister_observer(observer)
        }
    };
}

impl<T> Producer<T> {
    fn this_end(&self) -> &FifoInner<HeapRbProducer<T>> {
        &self.0.common.producer
    }

    fn peer_end(&self) -> &FifoInner<HeapRbConsumer<T>> {
        &self.0.common.consumer
    }

    fn update_pollee(&self) {
        // In theory, `rb.is_full()`/`rb.is_empty()`, where the `rb` is taken from either
        // `this_end` or `peer_end`, should reflect the same state. However, we need to take the
        // correct lock when updating the events to avoid races between the state check and the
        // event update.

        let this_end = self.this_end();
        let rb = this_end.rb();
        if self.is_shutdown() || self.is_peer_shutdown() {
            // The POLLOUT event is always set in this case. Don't try to remove it.
        } else if rb.is_full() {
            this_end.pollee.del_events(IoEvents::OUT);
        }
        drop(rb);

        let peer_end = self.peer_end();
        let rb = peer_end.rb();
        if !rb.is_empty() {
            peer_end.pollee.add_events(IoEvents::IN);
        }
        drop(rb);
    }

    impl_common_methods_for_channel!();
}

impl<T> Pollable for Producer<T> {
    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.poll(mask, poller)
    }
}

impl<T: Copy> Producer<T> {
    pub fn write(&self, buf: &[T]) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_write(buf)
        } else {
            // The POLLOUT event is set after shutdown, so waiting for the single event is enough.
            self.wait_events(IoEvents::OUT, || self.try_write(buf))
        }
    }

    fn try_write(&self, buf: &[T]) -> Result<usize> {
        if buf.is_empty() {
            // Even after shutdown, writing an empty buffer is still fine.
            return Ok(0);
        }

        if self.is_shutdown() || self.is_peer_shutdown() {
            return_errno_with_message!(Errno::EPIPE, "the channel is shut down");
        }

        let written_len = self.0.write(buf);
        self.update_pollee();

        if written_len > 0 {
            Ok(written_len)
        } else {
            return_errno_with_message!(Errno::EAGAIN, "the channel is full");
        }
    }
}

impl<T> Producer<T> {
    /// Pushes an item into the producer.
    ///
    /// On failure, this method returns `Err` containing
    /// the item fails to push.
    pub fn push(&self, item: T) -> core::result::Result<(), (Error, T)> {
        if self.is_nonblocking() {
            return self.try_push(item);
        }

        let mut stored_item = Some(item);

        // The POLLOUT event is set after shutdown, so waiting for the single event is enough.
        let result = self.wait_events(IoEvents::OUT, || {
            match self.try_push(stored_item.take().unwrap()) {
                Ok(()) => Ok(()),
                Err((err, item)) => {
                    stored_item = Some(item);
                    Err(err)
                }
            }
        });

        match result {
            Ok(()) => Ok(()),
            Err(err) => Err((err, stored_item.unwrap())),
        }
    }

    fn try_push(&self, item: T) -> core::result::Result<(), (Error, T)> {
        if self.is_shutdown() || self.is_peer_shutdown() {
            let err = Error::with_message(Errno::EPIPE, "the channel is shut down");
            return Err((err, item));
        }

        self.0.push(item).map_err(|item| {
            let err = Error::with_message(Errno::EAGAIN, "the channel is full");
            (err, item)
        })?;
        self.update_pollee();

        Ok(())
    }
}

impl<T> Drop for Producer<T> {
    fn drop(&mut self) {
        self.shutdown();

        // The POLLHUP event indicates that the write end is shut down.
        //
        // No need to take a lock. There is no race because no one is modifying this particular event.
        self.peer_end().pollee.add_events(IoEvents::HUP);
    }
}

impl<T> Consumer<T> {
    fn this_end(&self) -> &FifoInner<HeapRbConsumer<T>> {
        &self.0.common.consumer
    }

    fn peer_end(&self) -> &FifoInner<HeapRbProducer<T>> {
        &self.0.common.producer
    }

    fn update_pollee(&self) {
        // In theory, `rb.is_full()`/`rb.is_empty()`, where the `rb` is taken from either
        // `this_end` or `peer_end`, should reflect the same state. However, we need to take the
        // correct lock when updating the events to avoid races between the state check and the
        // event update.

        let this_end = self.this_end();
        let rb = this_end.rb();
        if rb.is_empty() {
            this_end.pollee.del_events(IoEvents::IN);
        }
        drop(rb);

        let peer_end = self.peer_end();
        let rb = peer_end.rb();
        if !rb.is_full() {
            peer_end.pollee.add_events(IoEvents::OUT);
        }
        drop(rb);
    }

    impl_common_methods_for_channel!();
}

impl<T> Pollable for Consumer<T> {
    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.poll(mask, poller)
    }
}

impl<T: Copy> Consumer<T> {
    pub fn read(&self, buf: &mut [T]) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_read(buf)
        } else {
            // The POLLHUP event is in `IoEvents::ALWAYS_POLL`, which is not specified again.
            self.wait_events(IoEvents::IN, || self.try_read(buf))
        }
    }

    fn try_read(&self, buf: &mut [T]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        // This must be recorded before the actual operation to avoid race conditions.
        let is_shutdown = self.is_shutdown() || self.is_peer_shutdown();

        let read_len = self.0.read(buf);
        self.update_pollee();

        if read_len > 0 {
            Ok(read_len)
        } else if is_shutdown {
            Ok(0)
        } else {
            return_errno_with_message!(Errno::EAGAIN, "the channel is empty");
        }
    }
}

impl<T> Consumer<T> {
    /// Pops an item from the consumer.
    pub fn pop(&self) -> Result<Option<T>> {
        if self.is_nonblocking() {
            self.try_pop()
        } else {
            // The POLLHUP event is in `IoEvents::ALWAYS_POLL`, which is not specified again.
            self.wait_events(IoEvents::IN, || self.try_pop())
        }
    }

    fn try_pop(&self) -> Result<Option<T>> {
        // This must be recorded before the actual operation to avoid race conditions.
        let is_shutdown = self.is_shutdown() || self.is_peer_shutdown();

        let item = self.0.pop();
        self.update_pollee();

        if let Some(item) = item {
            Ok(Some(item))
        } else if is_shutdown {
            Ok(None)
        } else {
            return_errno_with_message!(Errno::EAGAIN, "the channel is empty")
        }
    }
}

impl<T> Drop for Consumer<T> {
    fn drop(&mut self) {
        self.shutdown();

        // The POLLERR event indicates that the read end is closed (so any subsequent writes will
        // fail with an `EPIPE` error).
        //
        // The lock is taken because we are also adding the POLLOUT event, which may have races
        // with the event updates triggered by the writer.
        let peer_end = self.peer_end();
        let _rb = peer_end.rb();
        peer_end.pollee.add_events(IoEvents::ERR | IoEvents::OUT);
    }
}

struct Fifo<T, R: TRights> {
    common: Arc<Common<T>>,
    _rights: R,
}

impl<T, R: TRights> Fifo<T, R> {
    pub fn new(common: Arc<Common<T>>) -> Self {
        Self {
            common,
            _rights: R::new(),
        }
    }
}

impl<T: Copy, R: TRights> Fifo<T, R> {
    #[require(R > Read)]
    pub fn read(&self, buf: &mut [T]) -> usize {
        let mut rb = self.common.consumer.rb();
        rb.pop_slice(buf)
    }

    #[require(R > Write)]
    pub fn write(&self, buf: &[T]) -> usize {
        let mut rb = self.common.producer.rb();
        rb.push_slice(buf)
    }
}

impl<T, R: TRights> Fifo<T, R> {
    /// Pushes an item into the endpoint.
    /// If the `push` method failes, this method will return
    /// `Err` containing the item that hasn't been pushed
    #[require(R > Write)]
    pub fn push(&self, item: T) -> core::result::Result<(), T> {
        let mut rb = self.common.producer.rb();
        rb.push(item)
    }

    /// Pops an item from the endpoint.
    #[require(R > Read)]
    pub fn pop(&self) -> Option<T> {
        let mut rb = self.common.consumer.rb();
        rb.pop()
    }
}

struct Common<T> {
    producer: FifoInner<HeapRbProducer<T>>,
    consumer: FifoInner<HeapRbConsumer<T>>,
}

impl<T> Common<T> {
    fn with_capacity_and_flags(capacity: usize, flags: StatusFlags) -> Result<Self> {
        check_status_flags(flags)?;

        if capacity == 0 {
            return_errno_with_message!(Errno::EINVAL, "the channel capacity cannot be zero");
        }

        let rb: HeapRb<T> = HeapRb::new(capacity);
        let (rb_producer, rb_consumer) = rb.split();

        let producer = FifoInner::new(rb_producer, IoEvents::OUT, flags);
        let consumer = FifoInner::new(rb_consumer, IoEvents::empty(), flags);

        Ok(Self { producer, consumer })
    }

    pub fn capacity(&self) -> usize {
        self.producer.rb().capacity()
    }
}

struct FifoInner<T> {
    rb: Mutex<T>,
    pollee: Pollee,
    is_shutdown: AtomicBool,
    status_flags: AtomicU32,
}

impl<T> FifoInner<T> {
    pub fn new(rb: T, init_events: IoEvents, status_flags: StatusFlags) -> Self {
        Self {
            rb: Mutex::new(rb),
            pollee: Pollee::new(init_events),
            is_shutdown: AtomicBool::new(false),
            status_flags: AtomicU32::new(status_flags.bits()),
        }
    }

    pub fn rb(&self) -> MutexGuard<T> {
        self.rb.lock()
    }

    pub fn is_shutdown(&self) -> bool {
        self.is_shutdown.load(Ordering::Acquire)
    }

    pub fn shutdown(&self) {
        self.is_shutdown.store(true, Ordering::Release)
    }

    pub fn status_flags(&self) -> StatusFlags {
        let bits = self.status_flags.load(Ordering::Relaxed);
        StatusFlags::from_bits(bits).unwrap()
    }

    pub fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        check_status_flags(new_flags)?;
        self.status_flags.store(new_flags.bits(), Ordering::Relaxed);
        Ok(())
    }
}

fn check_status_flags(flags: StatusFlags) -> Result<()> {
    let valid_flags: StatusFlags = StatusFlags::O_NONBLOCK | StatusFlags::O_DIRECT;

    if !valid_flags.contains(flags) {
        // FIXME: Linux seems to silently ignore invalid flags. See
        // <https://man7.org/linux/man-pages/man2/fcntl.2.html>.
        return_errno_with_message!(Errno::EINVAL, "the flags are invalid");
    }

    if flags.contains(StatusFlags::O_DIRECT) {
        return_errno_with_message!(Errno::EINVAL, "the `O_DIRECT` flag is not supported");
    }

    Ok(())
}

#[cfg(ktest)]
mod test {
    use alloc::sync::Arc;
    use core::sync::atomic;

    use ostd::{prelude::*, sync::AtomicBits};

    use super::*;
    use crate::thread::{
        kernel_thread::{KernelThreadExt, ThreadOptions},
        Thread,
    };

    #[ktest]
    fn test_non_copy() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct NonCopy(Arc<usize>);

        let channel = Channel::with_capacity(16).unwrap();
        let (producer, consumer) = channel.split();

        let data = NonCopy(Arc::new(99));
        let expected_data = data.clone();

        for _ in 0..3 {
            producer.push(data.clone()).unwrap();
        }

        for _ in 0..3 {
            let data = consumer.pop().unwrap().unwrap();
            assert_eq!(data, expected_data);
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ordering {
        ProduceThenConsume,
        ConsumeThenProduce,
    }

    fn test_blocking<P, C>(produce: P, consume: C, ordering: Ordering)
    where
        P: Fn(Producer<u8>) + Sync + Send + 'static,
        C: Fn(Consumer<u8>) + Sync + Send + 'static,
    {
        let channel = Channel::with_capacity(1).unwrap();
        let (producer, consumer) = channel.split();

        // FIXME: `ThreadOptions::new` currently accepts `Fn`, forcing us to use `SpinLock` to gain
        // internal mutability. We should avoid this `SpinLock` by making `ThreadOptions::new`
        // accept `FnOnce`.
        let producer_with_lock = SpinLock::new(Some(producer));
        let consumer_with_lock = SpinLock::new(Some(consumer));

        let signal_producer = Arc::new(AtomicBool::new(false));
        let signal_consumer = signal_producer.clone();

        let producer = Thread::spawn_kernel_thread(ThreadOptions::new(move || {
            let producer = producer_with_lock.lock().take().unwrap();

            if ordering == Ordering::ConsumeThenProduce {
                while !signal_producer.load(atomic::Ordering::Relaxed) {
                    Thread::yield_now();
                }
            } else {
                signal_producer.store(true, atomic::Ordering::Relaxed);
            }

            produce(producer);
        }));

        let consumer = Thread::spawn_kernel_thread(ThreadOptions::new(move || {
            let consumer = consumer_with_lock.lock().take().unwrap();

            if ordering == Ordering::ProduceThenConsume {
                while !signal_consumer.load(atomic::Ordering::Relaxed) {
                    Thread::yield_now();
                }
            } else {
                signal_consumer.store(true, atomic::Ordering::Relaxed);
            }

            consume(consumer);
        }));

        producer.join();
        consumer.join();
    }

    #[ktest]
    fn test_read_empty() {
        test_blocking(
            |producer| {
                assert_eq!(producer.write(&[1]).unwrap(), 1);
            },
            |consumer| {
                let mut buf = [0; 1];
                assert_eq!(consumer.read(&mut buf).unwrap(), 1);
                assert_eq!(&buf, &[1]);
            },
            Ordering::ConsumeThenProduce,
        );
    }

    #[ktest]
    fn test_write_full() {
        test_blocking(
            |producer| {
                assert_eq!(producer.write(&[1, 2]).unwrap(), 1);
                assert_eq!(producer.write(&[2]).unwrap(), 1);
            },
            |consumer| {
                let mut buf = [0; 2];
                assert_eq!(consumer.read(&mut buf).unwrap(), 1);
                assert_eq!(&buf[..1], &[1]);
                assert_eq!(consumer.read(&mut buf).unwrap(), 1);
                assert_eq!(&buf[..1], &[2]);
            },
            Ordering::ProduceThenConsume,
        );
    }

    #[ktest]
    fn test_read_closed() {
        test_blocking(
            |producer| drop(producer),
            |consumer| {
                let mut buf = [0; 1];
                assert_eq!(consumer.read(&mut buf).unwrap(), 0);
            },
            Ordering::ConsumeThenProduce,
        );
    }

    #[ktest]
    fn test_write_closed() {
        test_blocking(
            |producer| {
                assert_eq!(producer.write(&[1, 2]).unwrap(), 1);
                assert_eq!(producer.write(&[2]).unwrap_err().error(), Errno::EPIPE);
            },
            |consumer| drop(consumer),
            Ordering::ProduceThenConsume,
        );
    }
}
