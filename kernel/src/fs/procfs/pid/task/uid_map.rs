// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use super::TidDirOps;
use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    process::{Process, Uid},
};

/// Represents the inode at `/proc/[pid]/task/[tid]/uid_map` (and also `/proc/[pid]/uid_map`).
#[expect(dead_code)]
pub struct UidMapFileOps(Arc<Process>);

impl UidMapFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.process_ref.clone();
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3402>
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
        let output = format!("{:>10} {:>10} {:>10}\n", 0, 0, u32::from(Uid::INVALID));
        Ok(output.into_bytes())
    }
}
