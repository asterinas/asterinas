// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{FileOps, ProcFileBuilder},
        vfs::inode::Inode,
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/cmdline` (and also `/proc/[pid]/cmdline`).
pub struct CmdlineFileOps(Arc<Process>);

impl CmdlineFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.process_ref.clone();
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3340>
        ProcFileBuilder::new(Self(process_ref), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CmdlineFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let vmar_guard = self.0.lock_vmar();
        let Some(init_stack_reader) = vmar_guard.init_stack_reader() else {
            // According to Linux behavior, return an empty string
            // if the process is a zombie process.
            return Ok(0);
        };

        init_stack_reader.argv(offset, writer)
    }
}
