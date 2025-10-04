// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    process::{Gid, Process},
};

/// Represents the inode at `/proc/[pid]/task/[tid]/gid_map` (and also `/proc/[pid]/gid_map`).
#[expect(dead_code)]
pub struct GidMapFileOps(Arc<Process>);

impl GidMapFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3386>
        ProcFileBuilder::new(Self(process_ref), mkmod!(a+r, u+w))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for GidMapFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        // This is the default GID map for the initial user namespace.
        // TODO: Retrieve the GID map from the user namespace of the current process
        // instead of returning this hard-coded value.
        let output = format!("{:>10} {:>10} {:>10}\n", 0, 0, u32::from(Gid::INVALID));
        Ok(output.into_bytes())
    }
}
