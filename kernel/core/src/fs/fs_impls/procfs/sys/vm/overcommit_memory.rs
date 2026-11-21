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

/// The default memory overcommit policy, `OVERCOMMIT_GUESS`.
///
/// Asterinas does not perform commit accounting and never rejects a mapping based
/// on a commit limit; the documented Linux default is exposed to user space for
/// compatibility.
///
/// Reference:
/// <https://elixir.bootlin.com/linux/v6.16.5/source/include/uapi/linux/mman.h#L13>
const OVERCOMMIT_GUESS: u32 = 0;

/// Represents the inode at `/proc/sys/vm/overcommit_memory`.
pub struct OvercommitMemoryFileOps;

impl OvercommitMemoryFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/mm/util.c#L750>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for OvercommitMemoryFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", OVERCOMMIT_GUESS)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/vm/overcommit_memory` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/sys/vm/overcommit_memory` is not supported"
        );
    }
}
