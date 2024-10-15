// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/uid_map`.
/// This file is used to map the UIDs of the process to the UIDs of the caller.
/// See https://man7.org/linux/man-pages/man5/proc_pid_uid_map.5.html for details.
/// FIXME: Implement the logic when we have the actual implementation of PID namespace.
pub struct UidMapFileOps;

impl UidMapFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for UidMapFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        Ok("0 dummy 1\n".to_string().into_bytes())
    }
}
