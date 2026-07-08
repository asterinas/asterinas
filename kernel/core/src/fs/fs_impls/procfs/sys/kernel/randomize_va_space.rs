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

/// The address-space layout randomization (ASLR) policy reported to user space.
///
/// Asterinas randomizes the user-space address-space layout (stack, heap, and
/// ELF load base) and does not support disabling it, which corresponds to
/// Linux's `2` (full randomization).
///
/// Reference:
/// <https://elixir.bootlin.com/linux/v6.16.5/source/mm/memory.c#L121>
const RANDOMIZE_VA_SPACE_FULL: u32 = 2;

/// Represents the inode at `/proc/sys/kernel/randomize_va_space`.
pub struct RandomizeVaSpaceFileOps;

impl RandomizeVaSpaceFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sysctl.c#L1716>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for RandomizeVaSpaceFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", RANDOMIZE_VA_SPACE_FULL)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/kernel/randomize_va_space` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/sys/kernel/randomize_va_space` is not supported"
        );
    }
}
