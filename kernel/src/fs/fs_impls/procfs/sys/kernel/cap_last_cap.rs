// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{FileOps, ProcFileBuilder},
        vfs::inode::Inode,
    },
    prelude::*,
    process::credentials::capabilities::CapSet,
};

/// Represents the inode at `/proc/sys/kernel/cap_last_cap`.
pub struct CapLastCapFileOps;

impl CapLastCapFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sysctl.c#L1701>
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CapLastCapFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let cap_last_cap_value = CapSet::most_significant_bit();
        writeln!(printer, "{}", cap_last_cap_value)?;

        Ok(printer.bytes_written())
    }
}
