// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        procfs::template::{FileOpsByHandle, ProcFileBuilder},
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
    vm::vmar::Vmar,
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

impl FileOpsByHandle for MemFileOps {
    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        let vmar = self.0.lock_vmar().as_weak();

        let handle = check_may_access(
            current_thread!().as_posix_thread().unwrap(),
            self.0.main_thread().as_posix_thread().unwrap(),
            PtraceMode::ATTACH_FSCREDS,
        )
        .map(|_| Box::new(MemFileHandle(self.0.clone(), vmar)) as Box<dyn FileIo>);

        Some(handle)
    }
}

struct MemFileHandle(Arc<Process>, Weak<Vmar>);

impl Pollable for MemFileHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for MemFileHandle {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let vmar_guard = self.0.lock_vmar();
        if !Weak::ptr_eq(&vmar_guard.as_weak(), &self.1) {
            return Ok(0);
        }
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
        if !Weak::ptr_eq(&vmar_guard.as_weak(), &self.1) {
            return Ok(0);
        }
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

impl FileIo for MemFileHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }
}
