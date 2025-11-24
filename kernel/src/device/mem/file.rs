// SPDX-License-Identifier: MPL-2.0

use alloc::vec;

use ostd::mm::{FallibleVmWrite, VmReader, VmWriter};

use crate::{
    error::Errno,
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::Result,
    process::signal::{PollHandle, Pollable},
    return_errno_with_message,
    util::random,
};

pub fn geturandom(writer: &mut VmWriter) -> Result<usize> {
    const IO_CAPABILITY: usize = 4096;

    if !writer.has_avail() {
        return Ok(0);
    }

    let mut buffer = vec![0; writer.avail().min(IO_CAPABILITY)];
    let mut written_bytes = 0;

    while writer.has_avail() {
        random::getrandom(&mut buffer[..writer.avail().min(IO_CAPABILITY)]);
        match writer.write_fallible(&mut VmReader::from(buffer.as_slice())) {
            Ok(len) => written_bytes += len,
            Err((err, 0)) if written_bytes == 0 => return Err(err.into()),
            Err((_, len)) => return Ok(written_bytes + len),
        }
    }

    Ok(written_bytes)
}

// TODO: Support true randomness by collecting environment noise.
pub use geturandom as getrandom;

#[derive(Debug, Copy, Clone)]
#[expect(dead_code)]
pub(super) enum MemFile {
    Mem,
    Kmem,
    Null,
    Port,
    Zero,
    Core,
    Full,
    Random,
    Urandom,
    Aio,
    Kmsg,
    Oldmem,
}

impl MemFile {
    pub(super) fn minor(&self) -> u32 {
        match self {
            MemFile::Mem => 1,
            MemFile::Kmem => 2,
            MemFile::Null => 3,
            MemFile::Port => 4,
            MemFile::Zero => 5,
            MemFile::Core => 6,
            MemFile::Full => 7,
            MemFile::Random => 8,
            MemFile::Urandom => 9,
            MemFile::Aio => 10,
            MemFile::Kmsg => 11,
            MemFile::Oldmem => 12,
        }
    }

    pub(super) fn name(&self) -> &str {
        match self {
            MemFile::Mem => "mem",
            MemFile::Kmem => "kmem",
            MemFile::Null => "null",
            MemFile::Port => "port",
            MemFile::Zero => "zero",
            MemFile::Core => "core",
            MemFile::Full => "full",
            MemFile::Random => "random",
            MemFile::Urandom => "urandom",
            MemFile::Aio => "aio",
            MemFile::Kmsg => "kmsg",
            MemFile::Oldmem => "oldmem",
        }
    }
}

impl Pollable for MemFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for MemFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        match self {
            MemFile::Full | MemFile::Zero => {
                let len = writer.avail();
                writer.fill_zeros(len)?;
                Ok(len)
            }
            MemFile::Null => Ok(0),
            MemFile::Random => getrandom(writer),
            MemFile::Urandom => geturandom(writer),
            _ => return_errno_with_message!(Errno::EINVAL, "read is not supported yet"),
        }
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        match self {
            MemFile::Null | MemFile::Random | MemFile::Urandom | MemFile::Zero => {
                let len = reader.remain();
                reader.skip(len);
                Ok(len)
            }
            MemFile::Full => {
                return_errno_with_message!(Errno::ENOSPC, "no space left on /dev/full")
            }
            _ => return_errno_with_message!(Errno::EINVAL, "write is not supported yet"),
        }
    }
}

impl FileIo for MemFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
