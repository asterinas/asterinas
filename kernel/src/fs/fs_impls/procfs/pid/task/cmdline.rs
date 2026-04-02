// SPDX-License-Identifier: MPL-2.0

use super::{TidDirOps, process_from_pid_entry};
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{FileOps, ProcFileBuilder},
        vfs::inode::Inode,
    },
    prelude::*,
    process::pid_table::PidEntry,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/cmdline` (and also `/proc/[pid]/cmdline`).
pub struct CmdlineFileOps(Arc<PidEntry>);

impl CmdlineFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3340>
        ProcFileBuilder::new(Self(dir.pid_entry().clone()), mkmod!(a+r))
            .parent(parent)
            .need_revalidation()
            .build()
            .unwrap()
    }
}

impl FileOps for CmdlineFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let Some(process) = process_from_pid_entry(&self.0) else {
            return_errno_with_message!(Errno::ESRCH, "the process has been reaped");
        };
        let vmar_guard = process.lock_vmar();
        let Some(init_stack_reader) = vmar_guard.init_stack_reader() else {
            // According to Linux behavior, return an empty string
            // if the process is a zombie process.
            return Ok(0);
        };

        init_stack_reader.argv(offset, writer)
    }
}
