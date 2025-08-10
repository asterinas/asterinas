// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    process::credentials::capabilities::CapSet,
};

/// Represents the inode at `/proc/sys/kernel/cap_last_cap`.
pub struct CapLastCapFileOps;

impl CapLastCapFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sysctl.c#L1701>
        ProcFileBuilder::new(Self, InodeMode::from_bits_truncate(0o444))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CapLastCapFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let cap_last_cap_value = CapSet::most_significant_bit();
        let output = format!("{}\n", cap_last_cap_value);
        Ok(output.into_bytes())
    }
}
