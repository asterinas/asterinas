// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    fs::utils::{Endpoint, EndpointState},
    prelude::*,
    process::{
        posix_thread::AsPosixThread,
        signal::{
            PollHandle, Pollable,
            constants::SIGPIPE,
            signals::user::{UserSignal, UserSignalKind},
        },
    },
    util::ring_buffer::{RbConsumer, RbProducer, RingBuffer},
};

const DEFAULT_PIPE_BUF_SIZE: usize = 65536;

/// Maximum number of bytes guaranteed to be written to a pipe atomically.
///
/// If the number of bytes to be written is less than the threshold, the write must be atomic.
/// A non-blocking atomic write may fail with `EAGAIN`, even if there is room for a partial write.
/// In other words, a partial write is not allowed for an atomic write.
///
/// For more details, see the description of `PIPE_BUF` in
/// <https://man7.org/linux/man-pages/man7/pipe.7.html>.
#[cfg(not(ktest))]
const PIPE_BUF: usize = 4096;
#[cfg(ktest)]
const PIPE_BUF: usize = 2;

pub(super) fn new_pair() -> (PipeReader, PipeWriter) {
    new_pair_with_capacity(DEFAULT_PIPE_BUF_SIZE)
}

pub(super) fn new_pair_with_capacity(capacity: usize) -> (PipeReader, PipeWriter) {
    let (producer, consumer) = RingBuffer::new(capacity).split();
    let (producer_state, consumer_state) =
        Endpoint::new_pair(EndpointState::default(), EndpointState::default());

    (
        PipeReader::new(consumer, consumer_state),
        PipeWriter::new(producer, producer_state),
    )
}

pub(super) struct PipeReader {
    consumer: Mutex<RbConsumer<u8>>,
    state: Endpoint<EndpointState>,
}

impl PipeReader {
    fn new(consumer: RbConsumer<u8>, state: Endpoint<EndpointState>) -> Self {
        Self {
            consumer: Mutex::new(consumer),
            state,
        }
    }

    pub(super) fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let read = || {
            let mut consumer = self.consumer.lock();
            consumer.read_fallible(writer)
        };

        self.state.read_with(read)
    }

    pub(super) fn peer_shutdown(&self) {
        self.state.peer_shutdown();
    }

    pub(super) fn peer_activate(&self) {
        self.state.peer_activate();
    }

    fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();
        if self.state.is_peer_shutdown() {
            events |= IoEvents::HUP;
        }
        if !self.consumer.lock().is_empty() {
            events |= IoEvents::IN;
        }
        events
    }
}

impl Pollable for PipeReader {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.state
            .poll_with(mask, poller, || self.check_io_events())
    }
}

pub(super) struct PipeWriter {
    producer: Mutex<RbProducer<u8>>,
    state: Endpoint<EndpointState>,
}

impl PipeWriter {
    fn new(producer: RbProducer<u8>, state: Endpoint<EndpointState>) -> Self {
        Self {
            producer: Mutex::new(producer),
            state,
        }
    }

    pub(super) fn try_write(&self, reader: &mut VmReader) -> Result<usize> {
        let write = || {
            let mut producer = self.producer.lock();
            if reader.remain() <= PIPE_BUF && producer.free_len() < reader.remain() {
                // No sufficient space for an atomic write
                return Ok(0);
            }
            producer.write_fallible(reader)
        };

        let res = self.state.write_with(write);
        if res.is_err_and(|e| e.error() == Errno::EPIPE) {
            if let Some(posix_thread) = current_thread!().as_posix_thread() {
                posix_thread.enqueue_signal(Box::new(UserSignal::new(
                    SIGPIPE,
                    UserSignalKind::Kill,
                    posix_thread.process().pid(),
                    posix_thread.credentials().ruid(),
                )));
            }
        }

        res
    }

    pub(super) fn shutdown(&self) {
        self.state.shutdown();
    }

    pub(super) fn activate(&self) {
        self.state.activate();
    }

    fn check_io_events(&self) -> IoEvents {
        if self.state.is_shutdown() {
            IoEvents::ERR | IoEvents::OUT
        } else if self.producer.lock().free_len() >= PIPE_BUF {
            IoEvents::OUT
        } else {
            IoEvents::empty()
        }
    }
}

impl Pollable for PipeWriter {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.state
            .poll_with(mask, poller, || self.check_io_events())
    }
}
