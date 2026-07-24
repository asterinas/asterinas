// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::random,
};

pub fn geturandom(writer: &mut VmWriter) -> Result<usize> {
    read_random_bytes_without_blocking(writer)
}

pub fn getrandom(writer: &mut VmWriter, mode: RandomReadMode) -> Result<usize> {
    if !writer.has_avail() {
        return Ok(0);
    }

    match mode {
        RandomReadMode::Blocking => random::wait_until_ready()?,
        RandomReadMode::Nonblocking => random::try_wait_until_ready()?,
    }

    read_random_bytes_without_blocking(writer)
}

#[derive(Clone, Copy, Debug)]
pub enum RandomReadMode {
    Blocking,
    Nonblocking,
}

fn read_random_bytes_without_blocking(writer: &mut VmWriter) -> Result<usize> {
    const IO_CAPABILITY: usize = 4096;

    if !writer.has_avail() {
        return Ok(0);
    }

    let mut buffer = vec![0; writer.avail().min(IO_CAPABILITY)];
    let mut written_bytes = 0;

    while writer.has_avail() {
        let len = writer.avail().min(IO_CAPABILITY);
        random::fill_insecure(&mut buffer[..len]);
        match writer.write_fallible(&mut VmReader::from(buffer.as_slice())) {
            Ok(len) => written_bytes += len,
            Err((err, 0)) if written_bytes == 0 => return Err(err.into()),
            Err((_, len)) => return Ok(written_bytes + len),
        }
    }

    Ok(written_bytes)
}

#[expect(dead_code)]
#[derive(Clone, Copy, Debug)]
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

    pub(super) fn name(&self) -> &'static str {
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
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        match self {
            MemFile::Random => random::poll(mask, poller),
            _ => (IoEvents::IN | IoEvents::OUT) & mask,
        }
    }
}

impl FileOps for MemFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        match self {
            MemFile::Full | MemFile::Zero => {
                let len = writer.avail();
                writer.fill_zeros(len)?;
                Ok(len)
            }
            MemFile::Null => Ok(0),
            MemFile::Random => {
                let mode = if status_flags.contains(StatusFlags::O_NONBLOCK) {
                    RandomReadMode::Nonblocking
                } else {
                    RandomReadMode::Blocking
                };
                getrandom(writer, mode)
            }
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

impl PerOpenFileOps for MemFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
