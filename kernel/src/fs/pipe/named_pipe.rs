// SPDX-License-Identifier: MPL-2.0

use core::{
    num::Wrapping,
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd::sync::WaitQueue;

use super::common::{PipeReader, PipeWriter};
use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{AccessMode, InodeIo, StatusFlags},
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

impl Pollable for NamedPipeHandle {
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

impl Drop for NamedPipeHandle {
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

impl InodeIo for NamedPipeHandle {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
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
        if status_flags.contains(StatusFlags::O_NONBLOCK) {
            self.try_write(reader)
        } else {
            self.wait_events(IoEvents::OUT, None, || self.try_write(reader))
        }
    }
}

impl FileIo for NamedPipeHandle {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a FIFO file")
    }

    fn is_offset_aware(&self) -> bool {
        false
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
        status_flags: StatusFlags,
    ) -> Result<Box<dyn FileIo>> {
        let mut pipe = self.pipe.lock();
        let pipe_obj = pipe.get_or_create_pipe_obj();

        let handle: Box<dyn FileIo> = match access_mode {
            AccessMode::O_RDONLY => {
                pipe.read_count += 1;

                let old_value = pipe_obj.num_reader.fetch_add(1, Ordering::Relaxed);
                if old_value == 0 {
                    pipe_obj.reader.peer_activate();
                    self.wait_queue.wake_all();
                }

                let has_writer = pipe_obj.num_writer.load(Ordering::Relaxed) > 0;
                let handle = NamedPipeHandle::new(pipe_obj, access_mode);

                if !status_flags.contains(StatusFlags::O_NONBLOCK) && !has_writer {
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
                let handle = NamedPipeHandle::new(pipe_obj, access_mode);

                if !has_reader {
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
struct NamedPipeInner {
    // Holding a weak reference here ensures that the pipe object (including
    // the buffer data) will be dropped when no handle is open on the named
    // pipe. This is consistent with Linux behavior.
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
