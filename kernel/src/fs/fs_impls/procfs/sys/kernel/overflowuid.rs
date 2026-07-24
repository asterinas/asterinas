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

/// The UID reported to user space when a real UID does not fit into the 16-bit
/// identifier expected by a legacy interface, defined by Linux as
/// `DEFAULT_OVERFLOWUID`.
///
/// Reference:
/// <https://elixir.bootlin.com/linux/v6.16.5/source/include/linux/highuid.h#L41>
const DEFAULT_OVERFLOWUID: u32 = 65534;

/// Represents the inode at `/proc/sys/kernel/overflowuid`.
pub struct OverflowUidFileOps;

impl OverflowUidFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sys.c#L167>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for OverflowUidFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", DEFAULT_OVERFLOWUID)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/kernel/overflowuid` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/sys/kernel/overflowuid` is not supported"
        );
    }
}
