// SPDX-License-Identifier: MPL-2.0

use ostd::sync::{Rcu, WaitQueue};

use super::pipe::{self, PipeReader, PipeWriter};
use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{AccessMode, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// A handle representing the reader end of a named pipe.
struct ReaderHandle {
    reader: Arc<PipeReader>,
}

impl Drop for ReaderHandle {
    fn drop(&mut self) {
        self.reader.state.peer_shutdown();
    }
}

/// A handle representing the writer end of a named pipe.
struct WriterHandle {
    writer: Arc<PipeWriter>,
}

impl Drop for WriterHandle {
    fn drop(&mut self) {
        self.writer.state.shutdown();
    }
}

/// A handle for an opened named pipe that encapsulates different access modes.
///
/// This enum represents the different ways a named pipe can be opened:
/// - Read-only: Only reading operations are allowed
/// - Write-only: Only writing operations are allowed  
/// - Read-write: Both reading and writing operations are allowed
enum NamedPipeHandle {
    Reader(Arc<ReaderHandle>),
    Writer(Arc<WriterHandle>),
    Both {
        reader: Arc<ReaderHandle>,
        writer: Arc<WriterHandle>,
    },
}

impl NamedPipeHandle {
    fn new(pipe: &NamedPipe, access_mode: AccessMode) -> Self {
        pipe.wait_queue.wake_all();

        match (access_mode.is_readable(), access_mode.is_writable()) {
            (true, false) => Self::Reader(pipe.get_or_create_reader_handle()),
            (false, true) => Self::Writer(pipe.get_or_create_writer_handle()),
            (true, true) => Self::Both {
                reader: pipe.get_or_create_reader_handle(),
                writer: pipe.get_or_create_writer_handle(),
            },
            (false, false) => unreachable!("access mode must be readable or writable"),
        }
    }
}

impl Pollable for NamedPipeHandle {
    fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        match self {
            Self::Reader(handle) => {
                handle
                    .reader
                    .state
                    .poll_with(mask, poller.as_deref_mut(), || {
                        handle.reader.check_io_events()
                    })
            }
            Self::Writer(handle) => {
                handle
                    .writer
                    .state
                    .poll_with(mask, poller.as_deref_mut(), || {
                        handle.writer.check_io_events()
                    })
            }
            Self::Both { reader, writer } => {
                reader
                    .reader
                    .state
                    .poll_with(mask, poller.as_deref_mut(), || {
                        reader.reader.check_io_events()
                    })
                    | writer
                        .writer
                        .state
                        .poll_with(mask, poller, || writer.writer.check_io_events())
            }
        }
    }
}

impl FileIo for NamedPipeHandle {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        match self {
            Self::Reader(handle) => handle.reader.try_read(writer),
            Self::Writer(_) => Err(Error::with_message(Errno::EBADF, "not opened for reading")),
            Self::Both { reader, .. } => reader.reader.try_read(writer),
        }
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        match self {
            Self::Reader(_) => Err(Error::with_message(Errno::EBADF, "not opened for writing")),
            Self::Writer(handle) => handle.writer.try_write(reader),
            Self::Both { writer, .. } => writer.writer.try_write(reader),
        }
    }

    fn read_blocked(&self, writer: &mut VmWriter) -> Result<usize> {
        self.wait_events(IoEvents::IN, None, || self.read(writer))
    }

    fn write_blocked(&self, reader: &mut VmReader) -> Result<usize> {
        self.wait_events(IoEvents::OUT, None, || self.write(reader))
    }
}

/// A named pipe (FIFO) that provides inter-process communication.
///
/// Named pipes are special files that appear in the filesystem and provide
/// a communication channel between processes. It can be opened multiple times
/// for reading, writing, or both.
pub struct NamedPipe {
    reader: Arc<PipeReader>,
    writer: Arc<PipeWriter>,

    reader_handle: Rcu<Weak<ReaderHandle>>,
    writer_handle: Rcu<Weak<WriterHandle>>,
    wait_queue: WaitQueue,
}

impl NamedPipe {
    pub fn new() -> Result<Self> {
        let (reader, writer) = pipe::new_pair();

        Ok(Self {
            reader: Arc::new(reader),
            writer: Arc::new(writer),
            reader_handle: Rcu::new(Weak::new()),
            writer_handle: Rcu::new(Weak::new()),
            wait_queue: WaitQueue::new(),
        })
    }

    /// Opens the named pipe with the specified access mode and status flags.
    ///
    /// Returns a handle that implements `FileIo` for performing I/O operations.
    ///
    /// The open behavior follows POSIX semantics:
    /// - Opening for read-only blocks until a writer opens the pipe.
    /// - Opening for write-only blocks until a reader opens the pipe.
    /// - Opening for read-write never blocks.
    pub fn open(
        &self,
        access_mode: AccessMode,
        states_flag: StatusFlags,
    ) -> Result<Arc<dyn FileIo>> {
        let handle = Arc::new(NamedPipeHandle::new(self, access_mode));

        if states_flag.contains(StatusFlags::O_NONBLOCK) {
            return Ok(handle);
        }

        if !access_mode.is_writable() && self.writer_handle.read().get().upgrade().is_none() {
            self.wait_queue.pause_until(|| {
                if self.writer_handle.read().get().upgrade().is_some() {
                    Some(())
                } else {
                    None
                }
            })?;
        }

        if !access_mode.is_readable() && self.reader_handle.read().get().upgrade().is_none() {
            self.wait_queue.pause_until(|| {
                if self.reader_handle.read().get().upgrade().is_some() {
                    Some(())
                } else {
                    None
                }
            })?;
        }

        Ok(handle)
    }

    fn get_or_create_reader_handle(&self) -> Arc<ReaderHandle> {
        let reader_handle = self.reader_handle.read().get().upgrade();
        if let Some(handle) = reader_handle {
            handle
        } else {
            let reader_handle = Arc::new(ReaderHandle {
                reader: self.reader.clone(),
            });

            self.reader_handle.update(Arc::downgrade(&reader_handle));
            self.reader.state.peer_activate();

            reader_handle
        }
    }

    fn get_or_create_writer_handle(&self) -> Arc<WriterHandle> {
        let writer_handle = self.writer_handle.read().get().upgrade();
        if let Some(handle) = writer_handle {
            handle
        } else {
            let writer_handle = Arc::new(WriterHandle {
                writer: self.writer.clone(),
            });

            self.writer_handle.update(Arc::downgrade(&writer_handle));
            self.writer.state.activate();

            writer_handle
        }
    }
}
