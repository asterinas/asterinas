// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use super::{
    file_handle::FileLike,
    utils::{AccessMode, Channel, Consumer, InodeMode, InodeType, Metadata, Producer, StatusFlags},
};
use crate::{
    events::IoEvents,
    prelude::*,
    process::{
        signal::{PollHandle, Pollable},
        Gid, Uid,
    },
    time::clocks::RealTimeCoarseClock,
};

const DEFAULT_PIPE_BUF_SIZE: usize = 65536;

pub fn new_pair() -> Result<(Arc<PipeReader>, Arc<PipeWriter>)> {
    let (producer, consumer) = Channel::with_capacity(DEFAULT_PIPE_BUF_SIZE).split();

    Ok((
        PipeReader::new(consumer, StatusFlags::empty())?,
        PipeWriter::new(producer, StatusFlags::empty())?,
    ))
}

pub fn new_pair_with_capacity(capacity: usize) -> Result<(Arc<PipeReader>, Arc<PipeWriter>)> {
    let (producer, consumer) = Channel::with_capacity(capacity).split();

    Ok((
        PipeReader::new(consumer, StatusFlags::empty())?,
        PipeWriter::new(producer, StatusFlags::empty())?,
    ))
}

pub struct PipeReader {
    consumer: Consumer<u8>,
    status_flags: AtomicU32,
}

impl PipeReader {
    pub fn new(consumer: Consumer<u8>, status_flags: StatusFlags) -> Result<Arc<Self>> {
        check_status_flags(status_flags)?;

        Ok(Arc::new(Self {
            consumer,
            status_flags: AtomicU32::new(status_flags.bits()),
        }))
    }
}

impl Pollable for PipeReader {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.consumer.poll(mask, poller)
    }
}

impl FileLike for PipeReader {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let read_len = if self.status_flags().contains(StatusFlags::O_NONBLOCK) {
            self.consumer.try_read(writer)?
        } else {
            self.wait_events(IoEvents::IN, None, || self.consumer.try_read(writer))?
        };
        Ok(read_len)
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
            mode: InodeMode::from_bits_truncate(0o400),
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }
}

pub struct PipeWriter {
    producer: Producer<u8>,
    status_flags: AtomicU32,
}

impl PipeWriter {
    pub fn new(producer: Producer<u8>, status_flags: StatusFlags) -> Result<Arc<Self>> {
        check_status_flags(status_flags)?;

        Ok(Arc::new(Self {
            producer,
            status_flags: AtomicU32::new(status_flags.bits()),
        }))
    }
}

impl Pollable for PipeWriter {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.producer.poll(mask, poller)
    }
}

impl FileLike for PipeWriter {
    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if self.status_flags().contains(StatusFlags::O_NONBLOCK) {
            self.producer.try_write(reader)
        } else {
            self.wait_events(IoEvents::OUT, None, || self.producer.try_write(reader))
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
            mode: InodeMode::from_bits_truncate(0o200),
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

#[cfg(ktest)]
mod test {
    use alloc::sync::Arc;
    use core::sync::atomic::{self, AtomicBool};

    use ostd::prelude::*;

    use super::*;
    use crate::{
        fs::utils::Channel,
        thread::{kernel_thread::ThreadOptions, Thread},
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ordering {
        WriteThenRead,
        ReadThenWrite,
    }

    fn test_blocking<W, R>(write: W, read: R, ordering: Ordering)
    where
        W: Fn(Arc<PipeWriter>) + Sync + Send + 'static,
        R: Fn(Arc<PipeReader>) + Sync + Send + 'static,
    {
        let channel = Channel::with_capacity(2);
        let (writer, readr) = channel.split();

        let writer = PipeWriter::new(writer, StatusFlags::empty()).unwrap();
        let reader = PipeReader::new(readr, StatusFlags::empty()).unwrap();

        // FIXME: `ThreadOptions::new` currently accepts `Fn`, forcing us to use `SpinLock` to gain
        // internal mutability. We should avoid this `SpinLock` by making `ThreadOptions::new`
        // accept `FnOnce`.
        let writer_with_lock: SpinLock<_> = SpinLock::new(Some(writer));
        let reader_with_lock: SpinLock<_> = SpinLock::new(Some(reader));

        let signal_writer = Arc::new(AtomicBool::new(false));
        let signal_reader = signal_writer.clone();

        let writer = ThreadOptions::new(move || {
            let writer = writer_with_lock.lock().take().unwrap();

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
            let reader = reader_with_lock.lock().take().unwrap();

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
