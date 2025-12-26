// SPDX-License-Identifier: MPL-2.0

use core::{
    num::Wrapping,
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd::sync::WaitQueue;

use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{AccessMode, Endpoint, EndpointState, InodeIo, StatusFlags},
    },
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

/// A handle for a pipe that implements `FileIo`.
///
/// Once a handle for a `Pipe` exists, the corresponding pipe object will
/// not be dropped.
pub(in crate::fs) struct PipeHandle {
    inner: Arc<PipeObj>,
    access_mode: AccessMode,
}

impl PipeHandle {
    fn new(inner: Arc<PipeObj>, access_mode: AccessMode) -> Box<Self> {
        Box::new(Self { inner, access_mode })
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        // `InodeHandle` checks the access mode before calling methods in `FileIo`.
        debug_assert!(self.access_mode.is_readable());

        self.inner.reader.try_read(writer)
    }

    fn try_write(&self, reader: &mut VmReader) -> Result<usize> {
        // `InodeHandle` checks the access mode before calling methods in `FileIo`.
        debug_assert!(self.access_mode.is_writable());

        self.inner.writer.try_write(reader)
    }
}

impl Pollable for PipeHandle {
    fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.access_mode.is_readable() {
            events |= self.inner.reader.poll(mask, poller.as_deref_mut());
        }

        if self.access_mode.is_writable() {
            events |= self.inner.writer.poll(mask, poller);
        }

        events
    }
}

impl Drop for PipeHandle {
    fn drop(&mut self) {
        if self.access_mode.is_readable() {
            let old_value = self.inner.num_reader.fetch_sub(1, Ordering::Relaxed);
            if old_value == 1 {
                self.inner.reader.peer_shutdown();
            }
        }

        if self.access_mode.is_writable() {
            let old_value = self.inner.num_writer.fetch_sub(1, Ordering::Relaxed);
            if old_value == 1 {
                self.inner.writer.shutdown();
            }
        }
    }
}

impl InodeIo for PipeHandle {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        if !writer.has_avail() {
            // Even the peer endpoint has been closed, reading an empty buffer is
            // still fine.
            return Ok(0);
        }

        if status_flags.contains(StatusFlags::O_NONBLOCK) {
            self.try_read(writer)
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_read(writer))
        }
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        if !reader.has_remain() {
            // Even the peer endpoint has been closed, writing an empty buffer is
            // still fine.
            return Ok(0);
        }

        if status_flags.contains(StatusFlags::O_NONBLOCK) {
            self.try_write(reader)
        } else {
            self.wait_events(IoEvents::OUT, None, || self.try_write(reader))
        }
    }
}

impl FileIo for PipeHandle {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a FIFO file")
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}

/// A pipe (FIFO) that provides inter-process communication.
///
/// Pipes are special files that appear in the filesystem and provide a
/// communication channel between processes. It can be opened multiple times
/// for reading, writing, or both.
///
/// A `Pipe` will maintain exactly one **pipe object** that provides actual pipe
/// functionalities when there is at least one handle opened on it.
pub(in crate::fs) struct Pipe {
    pipe: Mutex<PipeInner>,
    wait_queue: WaitQueue,
}

impl Pipe {
    /// Creates a new pipe.
    pub fn new() -> Self {
        Self {
            pipe: Mutex::new(PipeInner::default()),
            wait_queue: WaitQueue::new(),
        }
    }

    /// Opens the named pipe with the specified access mode and status flags.
    ///
    /// Returns a handle that implements `FileIo` for performing I/O operations.
    ///
    /// The open behavior follows POSIX semantics:
    /// - Opening for read-only blocks until a writer opens the pipe.
    /// - Opening for write-only blocks until a reader opens the pipe.
    /// - Opening for read-write never blocks.
    ///
    /// If no handle of this named pipe has existed, the method will create a new pipe object.
    /// Otherwise, it will return a handle that works on the existing pipe object.
    pub fn open_named(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Box<dyn FileIo>> {
        self.open_handle(access_mode, status_flags, true)
    }

    /// Opens the anonymous pipe with the specified access mode and status flags.
    pub(super) fn open_anon(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Box<dyn FileIo>> {
        self.open_handle(access_mode, status_flags, false)
    }

    /// Opens the pipe and returns a `PipeHandle`.
    fn open_handle(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
        is_named_pipe: bool,
    ) -> Result<Box<dyn FileIo>> {
        check_status_flags(status_flags)?;

        let mut pipe = self.pipe.lock();
        let pipe_obj = pipe.get_or_create_pipe_obj();

        let handle = match access_mode {
            AccessMode::O_RDONLY => {
                pipe.read_count += 1;

                let old_value = pipe_obj.num_reader.fetch_add(1, Ordering::Relaxed);
                if old_value == 0 {
                    pipe_obj.reader.peer_activate();
                    self.wait_queue.wake_all();
                }

                let has_writer = pipe_obj.num_writer.load(Ordering::Relaxed) > 0;
                let handle = PipeHandle::new(pipe_obj, access_mode);

                // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/pipe.c#L1166-L1175>
                if is_named_pipe && !status_flags.contains(StatusFlags::O_NONBLOCK) && !has_writer {
                    let old_write_count = pipe.write_count;
                    drop(pipe);
                    self.wait_queue.pause_until(|| {
                        (old_write_count != self.pipe.lock().write_count).then_some(())
                    })?;
                }

                handle
            }
            AccessMode::O_WRONLY => {
                pipe.write_count += 1;

                let old_num_writer = pipe_obj.num_writer.fetch_add(1, Ordering::Relaxed);
                if old_num_writer == 0 {
                    pipe_obj.writer.activate();
                    self.wait_queue.wake_all();
                }

                let has_reader = pipe_obj.num_reader.load(Ordering::Relaxed) > 0;
                let handle = PipeHandle::new(pipe_obj, access_mode);

                // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/pipe.c#L1184-L1195>
                if is_named_pipe && !has_reader {
                    if status_flags.contains(StatusFlags::O_NONBLOCK) {
                        return_errno_with_message!(Errno::ENXIO, "no reader is present");
                    }

                    let old_read_count = pipe.read_count;
                    drop(pipe);
                    self.wait_queue.pause_until(|| {
                        (old_read_count != self.pipe.lock().read_count).then_some(())
                    })?;
                }

                handle
            }
            AccessMode::O_RDWR => {
                pipe.read_count += 1;
                pipe.write_count += 1;

                let old_num_reader = pipe_obj.num_reader.fetch_add(1, Ordering::Relaxed);
                let old_num_writer = pipe_obj.num_writer.fetch_add(1, Ordering::Relaxed);
                if old_num_reader == 0 || old_num_writer == 0 {
                    self.wait_queue.wake_all();
                    pipe_obj.writer.activate();
                }

                PipeHandle::new(pipe_obj, access_mode)
            }
        };

        Ok(handle)
    }
}

pub(in crate::fs) fn check_status_flags(status_flags: StatusFlags) -> Result<()> {
    if status_flags.contains(StatusFlags::O_DIRECT) {
        // TODO: Support "packet" mode for pipes.
        //
        // The `O_DIRECT` flag indicates that the pipe should operate in "packet" mode.
        // "O_DIRECT .. Older kernels that do not support this flag will indicate this via an
        // EINVAL error."
        //
        // See <https://man7.org/linux/man-pages/man2/pipe.2.html>.
        return_errno_with_message!(Errno::EINVAL, "the `O_DIRECT` flag is not supported");
    }

    // TODO: Setting most of the other flags will succeed on Linux, but their effects need to be
    // validated.

    Ok(())
}

struct PipeObj {
    reader: PipeReader,
    writer: PipeWriter,
    num_reader: AtomicUsize,
    num_writer: AtomicUsize,
}

impl PipeObj {
    fn new() -> Arc<Self> {
        let (reader, writer) = super::common::new_pair();
        Arc::new(Self {
            reader,
            writer,
            num_reader: AtomicUsize::new(0),
            num_writer: AtomicUsize::new(0),
        })
    }
}

#[derive(Default)]
struct PipeInner {
    // Holding a weak reference here ensures that the pipe object (including
    // the buffer data) will be dropped when no handle is open on the pipe.
    // This is consistent with Linux behavior.
    pipe_obj: Weak<PipeObj>,
    read_count: Wrapping<usize>,
    write_count: Wrapping<usize>,
}

impl PipeInner {
    fn get_or_create_pipe_obj(&mut self) -> Arc<PipeObj> {
        if let Some(pipe_obj) = self.pipe_obj.upgrade() {
            return pipe_obj;
        }

        let pipe_obj = PipeObj::new();
        self.pipe_obj = Arc::downgrade(&pipe_obj);

        pipe_obj
    }
}

#[cfg(not(ktest))]
const DEFAULT_PIPE_BUF_SIZE: usize = 65536;
#[cfg(ktest)]
const DEFAULT_PIPE_BUF_SIZE: usize = 2;

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

fn new_pair() -> (PipeReader, PipeWriter) {
    new_pair_with_capacity(DEFAULT_PIPE_BUF_SIZE)
}

fn new_pair_with_capacity(capacity: usize) -> (PipeReader, PipeWriter) {
    let (producer, consumer) = RingBuffer::new(capacity).split();
    let (producer_state, consumer_state) =
        Endpoint::new_pair(EndpointState::default(), EndpointState::default());

    (
        PipeReader::new(consumer, consumer_state),
        PipeWriter::new(producer, producer_state),
    )
}

struct PipeReader {
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

    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let read = || {
            let mut consumer = self.consumer.lock();
            consumer.read_fallible(writer)
        };

        self.state.read_with(read)
    }

    fn peer_shutdown(&self) {
        self.state.peer_shutdown();
    }

    fn peer_activate(&self) {
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

struct PipeWriter {
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

    fn try_write(&self, reader: &mut VmReader) -> Result<usize> {
        let write = || {
            let mut producer = self.producer.lock();
            if reader.remain() <= PIPE_BUF && producer.free_len() < reader.remain() {
                // No sufficient space for an atomic write
                return Ok(0);
            }
            producer.write_fallible(reader)
        };

        let res = self.state.write_with(write);
        if res.is_err_and(|e| e.error() == Errno::EPIPE)
            && let Some(posix_thread) = current_thread!().as_posix_thread()
        {
            posix_thread.enqueue_signal(Box::new(UserSignal::new(
                SIGPIPE,
                UserSignalKind::Kill,
                posix_thread.process().pid(),
                posix_thread.credentials().ruid(),
            )));
        }

        res
    }

    fn shutdown(&self) {
        self.state.shutdown();
    }

    fn activate(&self) {
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

#[cfg(ktest)]
mod test {
    use alloc::sync::Arc;
    use core::sync::atomic::{self, AtomicBool};

    use ostd::prelude::*;

    use super::*;
    use crate::thread::{Thread, kernel_thread::ThreadOptions};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ordering {
        WriteThenRead,
        ReadThenWrite,
    }

    fn test_blocking<W, R>(write: W, read: R, ordering: Ordering)
    where
        W: FnOnce(Box<dyn FileIo>) + Send + 'static,
        R: FnOnce(Box<dyn FileIo>) + Send + 'static,
    {
        let pipe = Pipe::new();
        let reader = pipe
            .open_anon(AccessMode::O_RDONLY, StatusFlags::empty())
            .unwrap();
        let writer = pipe
            .open_anon(AccessMode::O_WRONLY, StatusFlags::empty())
            .unwrap();

        let signal_writer = Arc::new(AtomicBool::new(false));
        let signal_reader = signal_writer.clone();

        let writer = ThreadOptions::new(move || {
            if ordering == Ordering::ReadThenWrite {
                while !signal_writer.load(atomic::Ordering::Relaxed) {
                    Thread::yield_now();
                }
            } else {
                signal_writer.store(true, atomic::Ordering::Relaxed);
            }

            write(writer);
        })
        .spawn();

        let reader = ThreadOptions::new(move || {
            if ordering == Ordering::WriteThenRead {
                while !signal_reader.load(atomic::Ordering::Relaxed) {
                    Thread::yield_now();
                }
            } else {
                signal_reader.store(true, atomic::Ordering::Relaxed);
            }

            read(reader);
        })
        .spawn();

        writer.join();
        reader.join();
    }

    #[ktest]
    fn test_read_empty() {
        test_blocking(
            |writer| {
                assert_eq!(write(writer.as_ref(), &[1]).unwrap(), 1);
            },
            |reader| {
                let mut buf = [0; 1];
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 1);
                assert_eq!(&buf, &[1]);
            },
            Ordering::ReadThenWrite,
        );
    }

    #[ktest]
    fn test_write_full() {
        test_blocking(
            |writer| {
                assert_eq!(write(writer.as_ref(), &[1, 2, 3]).unwrap(), 2);
                assert_eq!(write(writer.as_ref(), &[2]).unwrap(), 1);
            },
            |reader| {
                let mut buf = [0; 3];
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 2);
                assert_eq!(&buf[..2], &[1, 2]);
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 1);
                assert_eq!(&buf[..1], &[2]);
            },
            Ordering::WriteThenRead,
        );
    }

    #[ktest]
    fn test_read_closed() {
        test_blocking(
            drop,
            |reader| {
                let mut buf = [0; 1];
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 0);
            },
            Ordering::ReadThenWrite,
        );
    }

    #[ktest]
    fn test_write_closed() {
        test_blocking(
            |writer| {
                assert_eq!(write(writer.as_ref(), &[1, 2, 3]).unwrap(), 2);
                assert_eq!(
                    write(writer.as_ref(), &[2]).unwrap_err().error(),
                    Errno::EPIPE
                );
            },
            drop,
            Ordering::WriteThenRead,
        );
    }

    #[ktest]
    fn test_write_atomicity() {
        test_blocking(
            |writer| {
                assert_eq!(write(writer.as_ref(), &[1]).unwrap(), 1);
                assert_eq!(write(writer.as_ref(), &[1, 2]).unwrap(), 2);
            },
            |reader| {
                let mut buf = [0; 3];
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 1);
                assert_eq!(&buf[..1], &[1]);
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 2);
                assert_eq!(&buf[..2], &[1, 2]);
            },
            Ordering::WriteThenRead,
        );
    }

    fn read(reader: &dyn FileIo, buf: &mut [u8]) -> crate::prelude::Result<usize> {
        reader.read_at(
            0,
            &mut VmWriter::from(buf).to_fallible(),
            StatusFlags::empty(),
        )
    }

    fn write(writer: &dyn FileIo, buf: &[u8]) -> crate::prelude::Result<usize> {
        writer.write_at(
            0,
            &mut VmReader::from(buf).to_fallible(),
            StatusFlags::empty(),
        )
    }
}
