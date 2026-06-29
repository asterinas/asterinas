// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    device::{registered_block_device_classes, registered_char_device_classes},
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/devices`.
pub struct DevicesFileOps;

impl DevicesFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/devices.c#L52>
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for DevicesFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "Character devices:")?;
        for device_class in registered_char_device_classes() {
            writeln!(
                printer,
                "{:3} {}",
                device_class.major(),
                device_class.name()
            )?;
        }

        writeln!(printer)?;
        writeln!(printer, "Block devices:")?;
        for device_class in registered_block_device_classes() {
            writeln!(
                printer,
                "{:3} {}",
                device_class.major(),
                device_class.name()
            )?;
        }

        Ok(printer.bytes_written())
    }
}
