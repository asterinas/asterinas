// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/cmdline` file support, which provides
//! information about arguments passed to the kernel at boot time.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_cmdline.5.html>

use aster_util::printer::VmPrinter;
use ostd::boot::boot_info;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
};

/// Represents the inode at `/proc/cmdline`.
pub struct CmdLineFileOps;

impl CmdLineFileOps {
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
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        // TODO: Parse additional kernel command line information with `bootconfig`.
        // See <https://docs.kernel.org/admin-guide/bootconfig.html> for details.
        writeln!(printer, "{}", boot_info().kernel_cmdline)?;

        Ok(printer.bytes_written())
    }
}
