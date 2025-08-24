// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{
            pid::util::PidOrTid,
            template::{FileOps, ProcFileBuilder},
        },
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/cmdline`or `/proc/[pid]/task/[tid]/cmdline`.
pub struct CmdlineFileOps(PidOrTid);

impl CmdlineFileOps {
    pub fn new_inode(pid_or_tid: PidOrTid, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(pid_or_tid))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CmdlineFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let cmdline_output = if self.0.process().status().is_zombie() {
            // Returns 0 characters for zombie process.
            Vec::new()
        } else {
            let Ok(argv_cstrs) = self.0.process().vm().init_stack_reader().argv() else {
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
