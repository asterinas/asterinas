// SPDX-License-Identifier: MPL-2.0

use super::{
    file_handle::FileLike,
    pipe::{self, PipeReader, PipeWriter},
};
use crate::{
    events::IoEvents,
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

pub struct NamedPipe {
    reader: Arc<PipeReader>,
    writer: Arc<PipeWriter>,
}

impl NamedPipe {
    pub fn new() -> Result<Self> {
        let (reader, writer) = pipe::new_pair()?;

        Ok(Self { reader, writer })
    }

    pub fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.reader.read(writer)
    }

    pub fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.writer.write(reader)
    }
}

impl Pollable for NamedPipe {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        warn!("Named pipe doesn't support poll now, return IoEvents::empty for now.");
        IoEvents::empty()
    }
}
