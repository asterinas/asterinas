// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/setgroups`.
/// See https://man7.org/linux/man-pages/man2/setgroups.2.html for details.
pub struct SetgroupsFileOps;

impl SetgroupsFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for SetgroupsFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        // Implement the logic to return the data for the setgroups file.
        // This could be a string representation of the setgroups status.
        // FIXME: Modify the return value when we have the actual implementation.
        Ok("allow\n".to_string().into_bytes())
    }
}
