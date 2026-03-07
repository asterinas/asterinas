// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, FileIo, StatusFlags, mkmod},
        procfs::template::{FileOpsByHandle, ProcFileBuilder},
        vfs::inode::{Inode, InodeIo},
    },
    prelude::*,
    process::{
        Process, VmarSnapshot,
        posix_thread::{AsPosixThread, alien_access::AlienAccessMode},
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

impl FileOpsByHandle for MemFileOps {
    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Result<Box<dyn FileIo>> {
        // Hold the process VMAR lock while checking access permissions and
        // taking the VMAR identity snapshot to prevent race conditions.
        let vmar_guard = self.0.lock_vmar();

        self.0
            .main_thread()
            .as_posix_thread()
            .unwrap()
            .check_alien_access_from(
                current_thread!().as_posix_thread().unwrap(),
                AlienAccessMode::ATTACH_WITH_FS_CREDS,
            )
            .map_err(|_| Error::with_message(Errno::EACCES, "alien access check denied"))?;

        let vmar = vmar_guard.snapshot();
        Ok(Box::new(MemFileHandle(self.0.clone(), vmar)))
    }
}

/// A file handle opened from `/proc/[pid]/task/[tid]/mem` (and also `/proc/[pid]/mem`).
struct MemFileHandle(Arc<Process>, VmarSnapshot);

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
        if !vmar_guard.is_same_as(&self.1) {
            // The process has executed a new program.
            return Ok(0);
        }
        let Some(vmar) = vmar_guard.as_ref() else {
            // The process has exited.
            return Ok(0);
        };

        match vmar.read_alien(offset, writer) {
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
        if !vmar_guard.is_same_as(&self.1) {
            // The process has executed a new program.
            return Ok(0);
        }
        let Some(vmar) = vmar_guard.as_ref() else {
            // The process has exited.
            return Ok(0);
        };

        match vmar.write_alien(offset, reader) {
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
