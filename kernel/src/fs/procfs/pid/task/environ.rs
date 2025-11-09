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

/// Represents the inode at `/proc/[pid]/task/[tid]/environ` (and also `/proc/[pid]/environ`).
pub struct EnvironFileOps(Arc<Process>);

impl EnvironFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.process_ref.clone();
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3324>
        ProcFileBuilder::new(Self(process_ref), mkmod!(u+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for EnvironFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let vmar_guard = self.0.lock_vmar();
        let Some(init_stack_reader) = vmar_guard.init_stack_reader() else {
            // According to Linux behavior, return an empty string
            // if the process is a zombie process.
            return Ok(Vec::new());
        };
        Ok(init_stack_reader
            .envp()
            // Should we return an empty string if an error occurs?
            .unwrap_or_else(|_| Vec::new()))
    }
}
