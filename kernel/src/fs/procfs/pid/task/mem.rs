// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{AccessMode, Inode, InodeIo, StatusFlags, mkmod},
    },
    prelude::*,
    process::{
        Process,
        posix_thread::{
            AsPosixThread,
            ptrace::{PtraceMode, check_may_access},
        },
        signal::{PollHandle, Pollable},
    },
};

/// Represents the inode at `/proc/[pid]/task/[tid]/mem` (and also `/proc/[pid]/mem`).
pub struct MemFileOps(Arc<Process>);

impl MemFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.process_ref.clone();
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3347>
        ProcFileBuilder::new(Self(process_ref), mkmod!(u+rw))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for MemFileOps {
    fn read_at(&self, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        unreachable!("should read via opened `MemFileHandle`")
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        unreachable!("should write via opened `MemFileHandle`")
    }

    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        if self.0.lock_vmar().as_ref().is_none() {
            return Some(Err(Error::with_message(
                Errno::EACCES,
                "the process has exited",
            )));
        }

        let handle = check_may_access(
            current_thread!().as_posix_thread().unwrap(),
            self.0.main_thread().as_posix_thread().unwrap(),
            PtraceMode::ATTACH_FSCREDS,
        )
        .map(|_| Box::new(MemFileHandle(self.0.clone())) as Box<dyn FileIo>);

        Some(handle)
    }
}

struct MemFileHandle(Arc<Process>);

impl InodeIo for MemFileHandle {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let vmar_guard = self.0.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return Ok(0);
        };
        match vmar.read_remote(offset, writer) {
            Ok(bytes) => Ok(bytes),
            Err((_, 0)) => Err(Error::new(Errno::EIO)),
            Err((_, bytes)) => Ok(bytes),
        }
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let vmar_guard = self.0.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return Ok(0);
        };
        match vmar.write_remote(offset, reader) {
            Ok(bytes) => Ok(bytes),
            Err((_, 0)) => Err(Error::new(Errno::EIO)),
            Err((_, bytes)) => Ok(bytes),
        }
    }
}

impl Pollable for MemFileHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for MemFileHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }
}
