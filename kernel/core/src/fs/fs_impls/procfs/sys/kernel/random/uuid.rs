// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    util::random::getrandom,
};

/// Represents the inode at `/proc/sys/kernel/random/uuid`.
pub struct UuidFileOps;

impl UuidFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/drivers/char/random.c#L1706>
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for UuidFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        // Generate a fresh random version-4 UUID on every read, as Linux does.
        //
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/lib/uuid.c#L33>
        let mut uuid = [0u8; 16];
        getrandom(&mut uuid);
        uuid[6] = (uuid[6] & 0x0f) | 0x40;
        uuid[8] = (uuid[8] & 0x3f) | 0x80;

        write!(
            printer,
            "{:02x}{:02x}{:02x}{:02x}-",
            uuid[0], uuid[1], uuid[2], uuid[3]
        )?;
        write!(printer, "{:02x}{:02x}-", uuid[4], uuid[5])?;
        write!(printer, "{:02x}{:02x}-", uuid[6], uuid[7])?;
        write!(printer, "{:02x}{:02x}-", uuid[8], uuid[9])?;
        writeln!(
            printer,
            "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            uuid[10], uuid[11], uuid[12], uuid[13], uuid[14], uuid[15]
        )?;

        Ok(printer.bytes_written())
    }
}
