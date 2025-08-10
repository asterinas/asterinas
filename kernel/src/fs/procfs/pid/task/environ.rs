// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/environ` (and also `/proc/[pid]/environ`).
pub struct EnvironFileOps(Arc<Process>);

impl EnvironFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3324>
        ProcFileBuilder::new(Self(process_ref), InodeMode::from_bits_truncate(0o400))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for EnvironFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let envp_output = if self.0.status().is_zombie() {
            // Returns 0 characters for zombie process.
            Vec::new()
        } else {
            let Ok(envp_cstrs) = self.0.init_stack_reader().envp() else {
                return Ok(Vec::new());
            };
            envp_cstrs
                .into_iter()
                .flat_map(|cstr| cstr.into_bytes_with_nul().into_iter())
                .collect()
        };
        Ok(envp_output)
    }
}
