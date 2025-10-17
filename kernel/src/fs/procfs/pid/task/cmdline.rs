// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/cmdline` (and also `/proc/[pid]/cmdline`).
pub struct CmdlineFileOps(Arc<Process>);

impl CmdlineFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3340>
        ProcFileBuilder::new(Self(process_ref), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CmdlineFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let vmar_guard = self.0.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            // According to Linux behavior, return an empty string
            // if the process is a zombie process.
            return Ok(Vec::new());
        };
        Ok(vmar
            .init_stack_reader()
            .argv()
            // Should we return an empty string if an error occurs?
            .unwrap_or_else(|_| Vec::new()))
    }
}
