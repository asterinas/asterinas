// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        pipe::common::{PipeReader, PipeWriter},
        pseudofs::{pipefs_singleton, PseudoInode},
        utils::{mkmod, AccessMode, Inode, InodeType, Metadata, StatusFlags},
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable},
        Gid, Uid,
    },
    time::clocks::RealTimeCoarseClock,
};

const DEFAULT_PIPE_BUF_SIZE: usize = 65536;

/// Creates a pair of connected pipe file handles with the default capacity.
pub fn new_file_pair() -> Result<(Arc<PipeReaderFile>, Arc<PipeWriterFile>)> {
    new_file_pair_with_capacity(DEFAULT_PIPE_BUF_SIZE)
}

fn new_file_pair_with_capacity(
    capacity: usize,
) -> Result<(Arc<PipeReaderFile>, Arc<PipeWriterFile>)> {
    let (reader, writer) = super::common::new_pair_with_capacity(capacity);

    let pseudo_inode = {
        Arc::new(PseudoInode::new(
            0,
            InodeType::NamedPipe,
            mkmod!(u+rw),
            Uid::new_root(),
            Gid::new_root(),
            aster_block::BLOCK_SIZE,
            Arc::downgrade(pipefs_singleton()),
        ))
    };

    Ok((
        PipeReaderFile::new(reader, StatusFlags::empty(), pseudo_inode.clone())?,
        PipeWriterFile::new(writer, StatusFlags::empty(), pseudo_inode)?,
    ))
}

/// A file handle for reading from a pipe.
pub struct PipeReaderFile {
    reader: PipeReader,
    status_flags: AtomicU32,
    pseudo_inode: Arc<dyn Inode>,
}

impl PipeReaderFile {
    fn new(
        reader: PipeReader,
        status_flags: StatusFlags,
        pseudo_inode: Arc<PseudoInode>,
    ) -> Result<Arc<Self>> {
        check_status_flags(status_flags)?;

        Ok(Arc::new(Self {
            reader,
            status_flags: AtomicU32::new(status_flags.bits()),
            pseudo_inode,
        }))
    }
}

impl Pollable for PipeReaderFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.reader.poll(mask, poller)
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

    fn inode(&self) -> &Arc<dyn Inode> {
        &self.pseudo_inode
    }
}

impl Drop for PipeReaderFile {
    fn drop(&mut self) {
        self.reader.peer_shutdown();
    }
}

/// A file handle for writing to a pipe.
pub struct PipeWriterFile {
    writer: PipeWriter,
    status_flags: AtomicU32,
    pseudo_inode: Arc<dyn Inode>,
}

impl PipeWriterFile {
    fn new(
        writer: PipeWriter,
        status_flags: StatusFlags,
        pseudo_inode: Arc<PseudoInode>,
    ) -> Result<Arc<Self>> {
        check_status_flags(status_flags)?;

        Ok(Arc::new(Self {
            writer,
            status_flags: AtomicU32::new(status_flags.bits()),
            pseudo_inode,
        }))
    }
}

impl Pollable for PipeWriterFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.writer.poll(mask, poller)
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

    fn inode(&self) -> &Arc<dyn Inode> {
        &self.pseudo_inode
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
        self.writer.shutdown();
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
