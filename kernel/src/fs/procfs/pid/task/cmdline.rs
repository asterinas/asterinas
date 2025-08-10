// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/cmdline` (and also `/proc/[pid]/cmdline`).
pub struct CmdlineFileOps(Arc<Process>);

impl CmdlineFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3340>
        ProcFileBuilder::new(Self(process_ref), InodeMode::from_bits_truncate(0o444))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CmdlineFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let cmdline_output = if self.0.status().is_zombie() {
            // Returns 0 characters for zombie process.
            Vec::new()
        } else {
            let Ok(argv_cstrs) = self.0.init_stack_reader().argv() else {
                return Ok(Vec::new());
            };
            argv_cstrs
                .into_iter()
                .flat_map(|c_str| c_str.into_bytes_with_nul().into_iter())
                .collect()
        };
        Ok(cmdline_output)
    }
}
