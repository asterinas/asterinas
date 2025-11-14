// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::{VmPrinter, VmPrinterError};

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        registry::FsProperties,
        utils::{mkmod, Inode},
    },
    prelude::*,
};

/// Represents the inode at /proc/filesystems.
pub struct FileSystemsFileOps;

impl FileSystemsFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/filesystems.c#L259>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for FileSystemsFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        crate::fs::registry::with_iter(|iter| -> core::result::Result<(), VmPrinterError> {
            for (fs_name, fs_type) in iter {
                if fs_type.properties().contains(FsProperties::NEED_DISK) {
                    writeln!(printer, "\t{}", fs_name)?;
                } else {
                    writeln!(printer, "nodev\t{}", fs_name)?;
                }
            }

            Ok(())
        })?;

        Ok(printer.bytes_written())
    }
}
