// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    process::posix_thread::PID_MAX,
};

/// Represents the inode at `/proc/sys/kernel/pid_max`.
pub struct PidMaxFileOps;

impl PidMaxFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for PidMaxFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let output = format!("{}\n", PID_MAX);
        Ok(output.into_bytes())
    }
}
