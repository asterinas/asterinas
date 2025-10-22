// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use super::{
    file_handle::FileLike,
    utils::{mkmod, AccessMode, Endpoint, EndpointState, InodeType, Metadata, StatusFlags},
};
use crate::{
    events::IoEvents,
    prelude::*,
    process::{
        posix_thread::AsPosixThread,
        signal::{
            constants::SIGPIPE,
            signals::user::{UserSignal, UserSignalKind},
            PollHandle, Pollable,
        },
        Gid, Uid,
    },
    time::clocks::RealTimeCoarseClock,
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

/// Creates a pair of connected pipe file handles with the default capacity.
pub fn new_file_pair() -> Result<(Arc<PipeReaderFile>, Arc<PipeWriterFile>)> {
    new_file_pair_with_capacity(DEFAULT_PIPE_BUF_SIZE)
}

pub(super) fn new_pair() -> (PipeReader, PipeWriter) {
    new_pair_with_capacity(DEFAULT_PIPE_BUF_SIZE)
}

fn new_file_pair_with_capacity(
    capacity: usize,
) -> Result<(Arc<PipeReaderFile>, Arc<PipeWriterFile>)> {
    let (reader, writer) = new_pair_with_capacity(capacity);

    Ok((
        PipeReaderFile::new(reader, StatusFlags::empty())?,
        PipeWriterFile::new(writer, StatusFlags::empty())?,
    ))
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

pub(super) struct PipeReader {
    consumer: Mutex<RbConsumer<u8>>,
    pub(super) state: Endpoint<EndpointState>,
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

    pub(super) fn check_io_events(&self) -> IoEvents {
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

impl Drop for PipeReaderFile {
    fn drop(&mut self) {
        self.reader.state.peer_shutdown();
    }
}

/// A file handle for reading from a pipe.
pub struct PipeReaderFile {
    reader: PipeReader,
    status_flags: AtomicU32,
}

impl PipeReaderFile {
    fn new(reader: PipeReader, status_flags: StatusFlags) -> Result<Arc<Self>> {
        check_status_flags(status_flags)?;

        Ok(Arc::new(Self {
            reader,
            status_flags: AtomicU32::new(status_flags.bits()),
        }))
    }
}

impl Pollable for PipeReaderFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.reader
            .state
            .poll_with(mask, poller, || self.reader.check_io_events())
    }
}

impl FileLike for PipeReaderFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if !writer.has_avail() {
            // Even the peer endpoint (`PipeWriter`) has been closed, reading an empty buffer is
            // still fine.
            return Ok(0);
        }

        if self.status_flags().contains(StatusFlags::O_NONBLOCK) {
            self.reader.try_read(writer)
        } else {
            self.wait_events(IoEvents::IN, None, || self.reader.try_read(writer))
        }
    }

    fn status_flags(&self) -> StatusFlags {
        StatusFlags::from_bits_truncate(self.status_flags.load(Ordering::Relaxed))
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        check_status_flags(new_flags)?;

        self.status_flags.store(new_flags.bits(), Ordering::Relaxed);
        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDONLY
    }

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "PipeFS" and link `PipeReader` to it.
        let now = RealTimeCoarseClock::get().read_time();
        Metadata {
            dev: 0,
            ino: 0,
            size: 0,
            blk_size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::NamedPipe,
            mode: mkmod!(u+r),
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }
}

pub(super) struct PipeWriter {
    producer: Mutex<RbProducer<u8>>,
    pub(super) state: Endpoint<EndpointState>,
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
            let thread = current_thread!();
            if let Some(posix_thread) = thread.as_posix_thread() {
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

    pub(super) fn check_io_events(&self) -> IoEvents {
        if self.state.is_shutdown() {
            IoEvents::ERR | IoEvents::OUT
        } else if self.producer.lock().free_len() >= PIPE_BUF {
            IoEvents::OUT
        } else {
            IoEvents::empty()
        }
    }
}

/// A file handle for writing to a pipe.
pub struct PipeWriterFile {
    writer: PipeWriter,
    status_flags: AtomicU32,
}

impl PipeWriterFile {
    fn new(writer: PipeWriter, status_flags: StatusFlags) -> Result<Arc<Self>> {
        check_status_flags(status_flags)?;

        Ok(Arc::new(Self {
            writer,
            status_flags: AtomicU32::new(status_flags.bits()),
        }))
    }
}

impl Pollable for PipeWriterFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.writer
            .state
            .poll_with(mask, poller, || self.writer.check_io_events())
    }
}

impl FileLike for PipeWriterFile {
    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if !reader.has_remain() {
            // Even the peer endpoint (`PipeReader`) has been closed, writing an empty buffer is
            // still fine.
            return Ok(0);
        }

        if self.status_flags().contains(StatusFlags::O_NONBLOCK) {
            self.writer.try_write(reader)
        } else {
            self.wait_events(IoEvents::OUT, None, || self.writer.try_write(reader))
        }
    }

    fn status_flags(&self) -> StatusFlags {
        StatusFlags::from_bits_truncate(self.status_flags.load(Ordering::Relaxed))
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        check_status_flags(new_flags)?;

        self.status_flags.store(new_flags.bits(), Ordering::Relaxed);
        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_WRONLY
    }

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "PipeFS" and link `PipeWriter` to it.
        let now = RealTimeCoarseClock::get().read_time();
        Metadata {
            dev: 0,
            ino: 0,
            size: 0,
            blk_size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::NamedPipe,
            mode: mkmod!(u+w),
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }
}

fn check_status_flags(status_flags: StatusFlags) -> Result<()> {
    if status_flags.contains(StatusFlags::O_DIRECT) {
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

impl Drop for PipeWriterFile {
    fn drop(&mut self) {
        self.writer.state.shutdown();
    }
}

#[cfg(ktest)]
mod test {
    use alloc::sync::Arc;
    use core::sync::atomic::{self, AtomicBool};

    use ostd::prelude::*;

    use super::*;
    use crate::thread::{kernel_thread::ThreadOptions, Thread};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ordering {
        WriteThenRead,
        ReadThenWrite,
    }

    fn test_blocking<W, R>(write: W, read: R, ordering: Ordering)
    where
        W: FnOnce(Arc<PipeWriterFile>) + Send + 'static,
        R: FnOnce(Arc<PipeReaderFile>) + Send + 'static,
    {
        let (reader, writer) = new_file_pair_with_capacity(2).unwrap();

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
                assert_eq!(writer.write(&mut reader_from(&[1])).unwrap(), 1);
            },
            |reader| {
                let mut buf = [0; 1];
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 1);
                assert_eq!(&buf, &[1]);
            },
            Ordering::ReadThenWrite,
        );
    }

    #[ktest]
    fn test_write_full() {
        test_blocking(
            |writer| {
                assert_eq!(writer.write(&mut reader_from(&[1, 2, 3])).unwrap(), 2);
                assert_eq!(writer.write(&mut reader_from(&[2])).unwrap(), 1);
            },
            |reader| {
                let mut buf = [0; 3];
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 2);
                assert_eq!(&buf[..2], &[1, 2]);
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 1);
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
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 0);
            },
            Ordering::ReadThenWrite,
        );
    }

    #[ktest]
    fn test_write_closed() {
        test_blocking(
            |writer| {
                assert_eq!(writer.write(&mut reader_from(&[1, 2, 3])).unwrap(), 2);
                assert_eq!(
                    writer.write(&mut reader_from(&[2])).unwrap_err().error(),
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
                assert_eq!(writer.write(&mut reader_from(&[1])).unwrap(), 1);
                assert_eq!(writer.write(&mut reader_from(&[1, 2])).unwrap(), 2);
            },
            |reader| {
                let mut buf = [0; 3];
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 1);
                assert_eq!(&buf[..1], &[1]);
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 2);
                assert_eq!(&buf[..2], &[1, 2]);
            },
            Ordering::WriteThenRead,
        );
    }

    fn reader_from(buf: &[u8]) -> VmReader {
        VmReader::from(buf).to_fallible()
    }

    fn writer_from(buf: &mut [u8]) -> VmWriter {
        VmWriter::from(buf).to_fallible()
    }
}
