// SPDX-License-Identifier: MPL-2.0

use super::{
    file_handle::FileLike,
    pipe::{self, PipeReader, PipeWriter},
    utils::{AccessMode, Metadata},
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

    pub fn with_capacity(capacity: usize) -> Result<Self> {
        let (reader, writer) = pipe::new_pair_with_capacity(capacity)?;

        Ok(Self { reader, writer })
    }
}

impl Pollable for NamedPipe {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        warn!("Named pipe doesn't support poll now, return IoEvents::empty for now.");
        IoEvents::empty()
    }
}

impl FileLike for NamedPipe {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.reader.read(writer)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.writer.write(reader)
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn metadata(&self) -> Metadata {
        self.reader.metadata()
    }
}
