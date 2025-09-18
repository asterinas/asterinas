// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/uid_map` (and also `/proc/[pid]/uid_map`).
#[expect(dead_code)]
pub struct UidMapFileOps(Arc<Process>);

impl UidMapFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3386>
        ProcFileBuilder::new(Self(process_ref), mkmod!(a+r, u+w))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for UidMapFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        // This is the default UID map for the initial user namespace.
        // TODO: Retrieve the UID map from the user namespace of the current process
        // instead of returning this hard-coded value.
        const INVALID_UID: u32 = u32::MAX;
        let res = format!("{:>10}{:>10}{:>10}", 0, 0, INVALID_UID);
        Ok(res.into_bytes())
    }
}
