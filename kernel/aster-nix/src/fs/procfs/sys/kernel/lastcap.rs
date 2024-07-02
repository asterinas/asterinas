// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    process::credentials::capabilities::CAP_LAST_CAP,
};

/// Represents the inode at `/proc/sys/kernel/cap_last_cap`.
pub struct CapLastCapFileOps;

impl CapLastCapFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for CapLastCapFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let cap_last_cap_value = CAP_LAST_CAP;
        let output = format!("{}\n", cap_last_cap_value);
        Ok(output.into_bytes())
    }
}
