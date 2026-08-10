// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// The GID reported to user space when a real GID does not fit into the 16-bit
/// identifier expected by a legacy interface, defined by Linux as
/// `DEFAULT_OVERFLOWGID`.
///
/// Reference:
/// <https://elixir.bootlin.com/linux/v6.16.5/source/include/linux/highuid.h#L42>
const DEFAULT_OVERFLOWGID: u32 = 65534;

/// Represents the inode at `/proc/sys/kernel/overflowgid`.
pub struct OverflowGidFileOps;

impl OverflowGidFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sys.c#L168>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for OverflowGidFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", DEFAULT_OVERFLOWGID)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/kernel/overflowgid` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/sys/kernel/overflowgid` is not supported"
        );
    }
}
