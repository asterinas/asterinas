// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    process::Process,
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
    fn data(&self) -> Result<Vec<u8>> {
        unreachable!()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let vmar_guard = self.0.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return_errno_with_message!(Errno::ESRCH, "the process has exited");
        };
        match vmar.read_remote(offset, writer) {
            Ok(bytes) => Ok(bytes),
            Err((err, 0)) => Err(err),
            Err((_, bytes)) => Ok(bytes),
        }
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let vmar_guard = self.0.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return_errno_with_message!(Errno::ESRCH, "the process has exited");
        };
        match vmar.write_remote(offset, reader) {
            Ok(bytes) => Ok(bytes),
            Err((err, 0)) => Err(err),
            Err((_, bytes)) => Ok(bytes),
        }
    }
}
