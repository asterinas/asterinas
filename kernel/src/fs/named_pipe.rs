// SPDX-License-Identifier: MPL-2.0

use core::{
    num::Wrapping,
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd::sync::WaitQueue;

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

/// A handle for a named pipe that implements `FileIo`.
///
/// Once a handle for a `NamedPipe` exists, the corresponding pipe object will
/// not be dropped.
struct NamedPipeHandle {
    inner: Arc<PipeObj>,
    access_mode: AccessMode,
}

impl NamedPipeHandle {
    fn new(inner: Arc<PipeObj>, access_mode: AccessMode) -> Arc<Self> {
        Arc::new(Self { inner, access_mode })
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        if let AccessMode::O_WRONLY = self.access_mode {
            return Err(Error::with_message(Errno::EBADF, "not opened for reading"));
        }

        self.inner.reader.try_read(writer)
    }

    fn try_write(&self, reader: &mut VmReader) -> Result<usize> {
        if let AccessMode::O_RDONLY = self.access_mode {
            return Err(Error::with_message(Errno::EBADF, "not opened for writing"));
        }

        self.inner.writer.try_write(reader)
    }
}

impl Pollable for NamedPipeHandle {
    fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        match self.access_mode {
            AccessMode::O_RDONLY => self
                .inner
                .reader
                .state
                .poll_with(mask, poller, || self.inner.reader.check_io_events()),
            AccessMode::O_WRONLY => self
                .inner
                .writer
                .state
                .poll_with(mask, poller, || self.inner.writer.check_io_events()),
            AccessMode::O_RDWR => {
                let read_events =
                    self.inner
                        .reader
                        .state
                        .poll_with(mask, poller.as_deref_mut(), || {
                            self.inner.reader.check_io_events()
                        });
                let write_events = self
                    .inner
                    .writer
                    .state
                    .poll_with(mask, poller, || self.inner.writer.check_io_events());

                read_events | write_events
            }
        }
    }
}

impl Drop for NamedPipeHandle {
    fn drop(&mut self) {
        match self.access_mode {
            AccessMode::O_RDONLY => {
                let old_value = self.inner.num_reader.fetch_sub(1, Ordering::Relaxed);
                if old_value == 1 {
                    self.inner.reader.state.peer_shutdown();
                }
            }
            AccessMode::O_WRONLY => {
                let old_value = self.inner.num_writer.fetch_sub(1, Ordering::Relaxed);
                if old_value == 1 {
                    self.inner.writer.state.shutdown();
                }
            }
            AccessMode::O_RDWR => {
                let old_reader_value = self.inner.num_reader.fetch_sub(1, Ordering::Relaxed);
                if old_reader_value == 1 {
                    self.inner.reader.state.peer_shutdown();
                }

                let old_writer_value = self.inner.num_writer.fetch_sub(1, Ordering::Relaxed);
                if old_writer_value == 1 {
                    self.inner.writer.state.shutdown();
                }
            }
        }
    }
}

impl FileIo for NamedPipeHandle {
    fn read(&self, writer: &mut VmWriter, status_flags: StatusFlags) -> Result<usize> {
        if let AccessMode::O_WRONLY = self.access_mode {
            return Err(Error::with_message(Errno::EBADF, "not opened for reading"));
        }

        if status_flags.contains(StatusFlags::O_NONBLOCK) {
            self.try_read(writer)
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_read(writer))
        }
    }

    fn write(&self, reader: &mut VmReader, status_flags: StatusFlags) -> Result<usize> {
        if let AccessMode::O_RDONLY = self.access_mode {
            return Err(Error::with_message(Errno::EBADF, "not opened for writing"));
        }

        if status_flags.contains(StatusFlags::O_NONBLOCK) {
            self.try_write(reader)
        } else {
            self.wait_events(IoEvents::OUT, None, || self.try_write(reader))
        }
    }
}

/// A named pipe (FIFO) that provides inter-process communication.
///
/// Named pipes are special files that appear in the filesystem and provide
/// a communication channel between processes. It can be opened multiple times
/// for reading, writing, or both.
///
/// A `NamedPipe` will maintain exactly one **pipe object** that provides actual pipe
/// functionalities when there is at least one handle opened on it.
pub struct NamedPipe {
    pipe: Mutex<NamedPipeInner>,
    wait_queue: WaitQueue,
}

impl NamedPipe {
    /// Creates a new named pipe.
    pub fn new() -> Result<Self> {
        Ok(Self {
            pipe: Mutex::new(NamedPipeInner::default()),
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
    ///
    /// If no handle of this named pipe has existed, the method will create a new pipe object.
    /// Otherwise, it will return a handle that works on the existing pipe object.
    pub fn open(
        &self,
        access_mode: AccessMode,
        status_flag: StatusFlags,
    ) -> Result<Arc<dyn FileIo>> {
        let mut pipe = self.pipe.lock();
        let pipe_obj = pipe.get_or_create_pipe_obj();

        let handle: Arc<dyn FileIo> = match access_mode {
            AccessMode::O_RDONLY => {
                pipe.read_count += 1;

                let old_value = pipe_obj.num_reader.fetch_add(1, Ordering::Relaxed);
                if old_value == 0 {
                    pipe_obj.reader.state.peer_activate();
                    self.wait_queue.wake_all();
                }

                let has_writer = pipe_obj.num_writer.load(Ordering::Relaxed) > 0;
                if !status_flag.contains(StatusFlags::O_NONBLOCK) && !has_writer {
                    let old_write_count = pipe.write_count;
                    drop(pipe);
                    self.wait_queue.pause_until(|| {
                        (old_write_count != self.pipe.lock().write_count).then_some(())
                    })?;
                }

                NamedPipeHandle::new(pipe_obj, access_mode)
            }
            AccessMode::O_WRONLY => {
                pipe.write_count += 1;

                let old_num_writer = pipe_obj.num_writer.fetch_add(1, Ordering::Relaxed);
                if old_num_writer == 0 {
                    pipe_obj.writer.state.activate();
                    self.wait_queue.wake_all();
                }

                let has_reader = pipe_obj.num_reader.load(Ordering::Relaxed) > 0;
                if !has_reader {
                    if status_flag.contains(StatusFlags::O_NONBLOCK) {
                        return_errno_with_message!(Errno::ENXIO, "no reader is present");
                    }

                    let old_read_count = pipe.read_count;
                    drop(pipe);
                    self.wait_queue.pause_until(|| {
                        (old_read_count != self.pipe.lock().read_count).then_some(())
                    })?;
                }

                NamedPipeHandle::new(pipe_obj, access_mode)
            }
            AccessMode::O_RDWR => {
                pipe.read_count += 1;
                pipe.write_count += 1;

                let old_num_reader = pipe_obj.num_reader.fetch_add(1, Ordering::Relaxed);
                let old_num_writer = pipe_obj.num_writer.fetch_add(1, Ordering::Relaxed);
                if old_num_reader == 0 || old_num_writer == 0 {
                    self.wait_queue.wake_all();
                }

                NamedPipeHandle::new(pipe_obj, access_mode)
            }
        };

        Ok(handle)
    }
}

struct PipeObj {
    reader: PipeReader,
    writer: PipeWriter,
    num_reader: AtomicUsize,
    num_writer: AtomicUsize,
}

impl PipeObj {
    fn new() -> Arc<Self> {
        let (reader, writer) = pipe::new_pair();
        Arc::new(Self {
            reader,
            writer,
            num_reader: AtomicUsize::new(0),
            num_writer: AtomicUsize::new(0),
        })
    }
}

#[derive(Default)]
struct NamedPipeInner {
    // `NamedPipe` does not directly own the pipe object. In this way, the pipe object
    // can be dropped when there is no handle opened on the named pipe, which is consistent
    // with the behavior of Linux.
    pipe_obj: Weak<PipeObj>,
    read_count: Wrapping<usize>,
    write_count: Wrapping<usize>,
}

impl NamedPipeInner {
    fn get_or_create_pipe_obj(&mut self) -> Arc<PipeObj> {
        if let Some(pipe_obj) = self.pipe_obj.upgrade() {
            return pipe_obj;
        }

        let pipe_obj = PipeObj::new();
        self.pipe_obj = Arc::downgrade(&pipe_obj);

        pipe_obj
    }
}
