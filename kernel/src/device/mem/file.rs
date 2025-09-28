// SPDX-License-Identifier: MPL-2.0

use alloc::vec;

use ostd::mm::{FallibleVmWrite, VmReader, VmWriter};

use crate::{
    error::Errno,
    events::IoEvents,
    fs::inode_handle::FileIo,
    prelude::Result,
    process::signal::{PollHandle, Pollable},
    return_errno_with_message,
};

pub fn getrandom(buf: &mut [u8]) -> Result<usize> {
    crate::util::random::getrandom(buf)?;
    Ok(buf.len())
}

pub use getrandom as geturandom;

#[derive(Debug)]
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

impl FileIo for MemFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let len = match self {
            MemFile::Full | MemFile::Zero => writer.fill_zeros(writer.avail())?,
            MemFile::Null => 0,
            MemFile::Random | MemFile::Urandom => {
                let mut buf = vec![0; writer.avail()];
                getrandom(buf.as_mut_slice())?;
                writer.write_fallible(&mut buf.as_slice().into())?
            }
            _ => return_errno_with_message!(Errno::EINVAL, "read is not supported yet"),
        };

        Ok(len)
    }

    fn write(&self, reader: &mut VmReader) -> crate::prelude::Result<usize> {
        match self {
            MemFile::Full => {
                return_errno_with_message!(Errno::ENOSPC, "no space left on /dev/full")
            }
            MemFile::Null | MemFile::Random | MemFile::Urandom | MemFile::Zero => {
                Ok(reader.remain())
            }
            _ => return_errno_with_message!(Errno::ENAVAIL, "write is not supported yet"),
        }
    }
}

impl Pollable for MemFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
