// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::sync::WaitQueue;

use super::pipe::{self, PipeReader, PipeWriter};
use crate::{
    events::IoEvents,
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

pub struct NamedPipe {
    reader: PipeReader,
    writer: PipeWriter,

    reader_count: AtomicUsize,
    writer_count: AtomicUsize,
    wait_queue: WaitQueue,
}

impl NamedPipe {
    pub fn new() -> Result<Self> {
        let (reader, writer) = pipe::new_pair();

        Ok(Self {
            reader,
            writer,
            reader_count: AtomicUsize::new(0),
            writer_count: AtomicUsize::new(0),
            wait_queue: WaitQueue::new(),
        })
    }

    /// Prepare to open the named pipe.
    pub fn prepare_open(&self, read: bool, write: bool, is_nonblocking: bool) -> Result<()> {
        if read {
            self.reader_count.fetch_add(1, Ordering::Relaxed);
        }
        if write {
            self.writer_count.fetch_add(1, Ordering::Relaxed);
        }

        self.wait_queue.wake_all();

        if !is_nonblocking && self.need_block() {
            self.wait_queue
                .pause_until(|| if self.need_block() { None } else { Some(()) })?;
        }

        Ok(())
    }

    /// Prepare to close the named pipe.
    pub fn prepare_close(&self, read: bool, write: bool) {
        if read {
            let old_value = self.reader_count.fetch_sub(1, Ordering::Relaxed);
            if old_value == 1 {
                // Wake up writers blocked on write
                self.reader.state.shutdown();
            }
        }
        if write {
            let old_value = self.writer_count.fetch_sub(1, Ordering::Relaxed);
            if old_value == 1 {
                // Wake up readers blocked on read
                self.writer.state.shutdown();
            }
        }
    }

    fn need_block(&self) -> bool {
        self.reader_count.load(Ordering::Relaxed) == 0
            || self.writer_count.load(Ordering::Relaxed) == 0
    }

    pub fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.reader.try_read(writer)
    }

    pub fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.writer.try_write(reader)
    }
}

impl Pollable for NamedPipe {
    fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        self.reader
            .state
            .poll_with(mask, poller.as_deref_mut(), || {
                self.reader.check_io_events()
            })
            | self
                .writer
                .state
                .poll_with(mask, poller, || self.writer.check_io_events())
    }
}
