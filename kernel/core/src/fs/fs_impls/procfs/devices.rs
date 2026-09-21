// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    device::registry::char,
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// Represents the inode at /proc/devices.
pub(super) struct DevicesFileOps;

impl DevicesFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/devices.c>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for DevicesFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "Character devices:")?;
        for (major, name) in char::major_devices() {
            writeln!(printer, "{:3} {}", major, name)?;
        }

        // Print an empty line to match the Linux format.
        writeln!(printer)?;

        writeln!(printer, "Block devices:")?;
        for (major, name) in aster_block::major_devices() {
            writeln!(printer, "{:3} {}", major, name)?;
        }

        Ok(printer.bytes_written())
    }
}

#[cfg(ktest)]
mod test {
    use device_id::MajorId;
    use ostd::prelude::ktest;

    use super::*;
    use crate::device::registry::char::acquire_major;

    fn read_devices() -> String {
        let mut buf = [0u8; 4096];
        let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
        let len = DevicesFileOps.read_at(0, &mut writer).unwrap();
        String::from_utf8_lossy(&buf[..len]).into_owned()
    }

    #[ktest]
    fn list_devices() {
        // Use major IDs outside the dynamic allocation ranges to avoid
        // conflicting with majors allocated by other tests.
        let char_owner = acquire_major(MajorId::new(42), "ktestchar").unwrap();
        let block_owner = aster_block::acquire_major(MajorId::new(300), "ktestblk").unwrap();

        let content = read_devices();
        assert!(content.starts_with("Character devices:\n"));
        assert!(content.contains(" 42 ktestchar\n"));
        assert!(content.contains("\n\nBlock devices:\n"));
        assert!(content.contains("300 ktestblk\n"));

        // Once the owners are dropped, the major IDs are released and
        // disappear from `/proc/devices`.
        drop(char_owner);
        drop(block_owner);

        let content = read_devices();
        assert!(!content.contains("ktestchar"));
        assert!(!content.contains("ktestblk"));
    }
}
