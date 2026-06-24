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

/// Represents the inode at `/proc/sys/fs/nr_open`.
pub struct NrOpenFileOps;

impl NrOpenFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Linux's default value is 1024 * 1024. virtiofsd reads this sysctl while raising
        // RLIMIT_NOFILE during startup.
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for NrOpenFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        const NR_OPEN: usize = 1024 * 1024;

        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", NR_OPEN)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/fs/nr_open` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/sys/fs/nr_open` is not supported"
        );
    }
}
