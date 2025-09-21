// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/cmdline` file support, which provides
//! information about arguments passed to the kernel at boot time.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_cmdline.5.html>

use alloc::format;

use ostd::boot::boot_info;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
};

/// Represents the inode at `/proc/cmdline`.
pub struct CmdLineFileOps;

impl CmdLineFileOps {
    /// Create a new inode for `/proc/cmdline`.
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/cmdline.c#L19>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CmdLineFileOps {
    /// Retrieve the data for `/proc/cmdline`.
    fn data(&self) -> Result<Vec<u8>> {
        // TODO: Parse additional kernel command line information with `bootconfig`.
        // See <https://docs.kernel.org/admin-guide/bootconfig.html> for details.
        let cmdline = format!("{}\n", boot_info().kernel_cmdline);
        Ok(cmdline.into_bytes())
    }
}
