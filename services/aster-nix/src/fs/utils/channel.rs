// SPDX-License-Identifier: MPL-2.0

use aster_rights_proc::require;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use ringbuf::{HeapConsumer as HeapRbConsumer, HeapProducer as HeapRbProducer, HeapRb};

use crate::events::IoEvents;
use crate::events::Observer;
use crate::prelude::*;
use crate::process::signal::{Pollee, Poller};
use aster_rights::{Read, ReadOp, TRights, Write, WriteOp};

use super::StatusFlags;

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
        let producer = Producer(EndPoint::new(common.clone(), WriteOp::new()));
        let consumer = Consumer(EndPoint::new(common, ReadOp::new()));
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

pub struct Producer<T>(EndPoint<T, WriteOp>);

pub struct Consumer<T>(EndPoint<T, ReadOp>);

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

        pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
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
        ) -> Result<Weak<dyn Observer<IoEvents>>> {
            self.this_end()
                .pollee
                .unregister_observer(observer)
                .ok_or_else(|| Error::with_message(Errno::ENOENT, "the observer is not registered"))
        }
    };
}

impl<T> Producer<T> {
    fn this_end(&self) -> &EndPointInner<HeapRbProducer<T>> {
        &self.0.common.producer
    }

    fn peer_end(&self) -> &EndPointInner<HeapRbConsumer<T>> {
        &self.0.common.consumer
    }

    fn update_pollee(&self) {
        let this_end = self.this_end();
        let peer_end = self.peer_end();

        // Update the event of pollee in a critical region so that pollee
        // always reflects the _true_ state of the underlying ring buffer
        // regardless of any race conditions.
        self.0.common.lock_event();

        let rb = this_end.rb();
        if rb.is_full() {
            this_end.pollee.del_events(IoEvents::OUT);
        }
        if !rb.is_empty() {
            peer_end.pollee.add_events(IoEvents::IN);
        }
    }

    impl_common_methods_for_channel!();
}

impl<T: Copy> Producer<T> {
    pub fn write(&self, buf: &[T]) -> Result<usize> {
        let is_nonblocking = self.is_nonblocking();

        // Fast path
        let res = self.try_write(buf);
        if should_io_return(&res, is_nonblocking) {
            return res;
        }

        // Slow path
        let mask = IoEvents::OUT;
        let poller = Poller::new();
        loop {
            let res = self.try_write(buf);
            if should_io_return(&res, is_nonblocking) {
                return res;
            }
            let events = self.poll(mask, Some(&poller));
            if events.is_empty() {
                // FIXME: should channel deal with timeout?
                poller.wait()?;
            }
        }
    }

    fn try_write(&self, buf: &[T]) -> Result<usize> {
        if self.is_shutdown() || self.is_peer_shutdown() {
            return_errno!(Errno::EPIPE);
        }

        if buf.is_empty() {
            return Ok(0);
        }

        let written_len = self.0.write(buf);

        self.update_pollee();

        if written_len > 0 {
            Ok(written_len)
        } else {
            return_errno_with_message!(Errno::EAGAIN, "try write later");
        }
    }
}

impl<T> Drop for Producer<T> {
    fn drop(&mut self) {
        self.shutdown();

        self.0.common.lock_event();

        // When reading from a channel such as a pipe or a stream socket,
        // POLLHUP merely indicates that the peer closed its end of the channel.
        self.peer_end().pollee.add_events(IoEvents::HUP);
    }
}

impl<T> Consumer<T> {
    fn this_end(&self) -> &EndPointInner<HeapRbConsumer<T>> {
        &self.0.common.consumer
    }

    fn peer_end(&self) -> &EndPointInner<HeapRbProducer<T>> {
        &self.0.common.producer
    }

    fn update_pollee(&self) {
        let this_end = self.this_end();
        let peer_end = self.peer_end();

        // Update the event of pollee in a critical region so that pollee
        // always reflects the _true_ state of the underlying ring buffer
        // regardless of any race conditions.
        self.0.common.lock_event();

        let rb = this_end.rb();
        if rb.is_empty() {
            this_end.pollee.del_events(IoEvents::IN);
        }
        if !rb.is_full() {
            peer_end.pollee.add_events(IoEvents::OUT);
        }
    }

    impl_common_methods_for_channel!();
}

impl<T: Copy> Consumer<T> {
    pub fn read(&self, buf: &mut [T]) -> Result<usize> {
        let is_nonblocking = self.is_nonblocking();

        // Fast path
        let res = self.try_read(buf);
        if should_io_return(&res, is_nonblocking) {
            return res;
        }

        // Slow path
        let mask = IoEvents::IN;
        let poller = Poller::new();
        loop {
            let res = self.try_read(buf);
            if should_io_return(&res, is_nonblocking) {
                return res;
            }
            let events = self.poll(mask, Some(&poller));
            if events.is_empty() {
                // FIXME: should channel have timeout?
                poller.wait()?;
            }
        }
    }

    fn try_read(&self, buf: &mut [T]) -> Result<usize> {
        if self.is_shutdown() {
            return_errno!(Errno::EPIPE);
        }
        if buf.is_empty() {
            return Ok(0);
        }

        let read_len = self.0.read(buf);

        self.update_pollee();

        if self.is_peer_shutdown() {
            return Ok(read_len);
        }

        if read_len > 0 {
            Ok(read_len)
        } else {
            return_errno_with_message!(Errno::EAGAIN, "try read later");
        }
    }
}

impl<T> Drop for Consumer<T> {
    fn drop(&mut self) {
        self.shutdown();

        self.0.common.lock_event();

        // POLLERR is also set for a file descriptor referring to the write end of a pipe
        // when the read end has been closed.
        self.peer_end().pollee.add_events(IoEvents::ERR);
    }
}

struct EndPoint<T, R: TRights> {
    common: Arc<Common<T>>,
    rights: R,
}

impl<T, R: TRights> EndPoint<T, R> {
    pub fn new(common: Arc<Common<T>>, rights: R) -> Self {
        Self { common, rights }
    }
}

impl<T: Copy, R: TRights> EndPoint<T, R> {
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

struct Common<T> {
    producer: EndPointInner<HeapRbProducer<T>>,
    consumer: EndPointInner<HeapRbConsumer<T>>,
    event_lock: Mutex<()>,
}

impl<T> Common<T> {
    fn with_capacity_and_flags(capacity: usize, flags: StatusFlags) -> Result<Self> {
        check_status_flags(flags)?;

        if capacity == 0 {
            return_errno_with_message!(Errno::EINVAL, "capacity cannot be zero");
        }

        let rb: HeapRb<T> = HeapRb::new(capacity);
        let (rb_producer, rb_consumer) = rb.split();

        let producer = EndPointInner::new(rb_producer, IoEvents::OUT, flags);
        let consumer = EndPointInner::new(rb_consumer, IoEvents::empty(), flags);
        let event_lock = Mutex::new(());

        Ok(Self {
            producer,
            consumer,
            event_lock,
        })
    }

    pub fn lock_event(&self) -> MutexGuard<()> {
        self.event_lock.lock()
    }

    pub fn capacity(&self) -> usize {
        self.producer.rb().capacity()
    }
}

struct EndPointInner<T> {
    rb: Mutex<T>,
    pollee: Pollee,
    is_shutdown: AtomicBool,
    status_flags: AtomicU32,
}

impl<T> EndPointInner<T> {
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
        return_errno_with_message!(Errno::EINVAL, "invalid flags");
    }
    if flags.contains(StatusFlags::O_DIRECT) {
        return_errno_with_message!(Errno::EINVAL, "O_DIRECT is not supported");
    }
    Ok(())
}

fn should_io_return(res: &Result<usize>, is_nonblocking: bool) -> bool {
    if is_nonblocking {
        return true;
    }
    match res {
        Ok(_) => true,
        Err(e) if e.error() == Errno::EAGAIN => false,
        Err(_) => true,
    }
}
