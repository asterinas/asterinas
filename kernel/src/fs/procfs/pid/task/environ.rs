// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/environ` (and also `/proc/[pid]/environ`).
pub struct EnvironFileOps(Arc<Process>);

impl EnvironFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3324>
        ProcFileBuilder::new(Self(process_ref), mkmod!(u+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for EnvironFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        Ok(self
            .0
            .init_stack_reader()
            .envp()
            // According to Linux behavior, return an empty string if an error occurs
            // (which is likely because the process is a zombie process).
            .unwrap_or_else(|_| Vec::new()))
    }
}
