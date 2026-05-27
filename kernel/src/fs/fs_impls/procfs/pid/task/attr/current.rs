// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/attr/current`.
pub(super) struct CurrentFileOps(TidDirOps);

impl CurrentFileOps {
    /// Creates the inode for `/proc/[pid]/attr/current`.
    pub(super) fn new_inode(dir: &super::AttrDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.tid_dir().clone()), parent, mkmod!(a+r))
    }
}

impl ProcFileOps for CurrentFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let Some(thread) = self.0.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };

        let mut printer = VmPrinter::new_skip(writer, offset);
        let label = thread
            .as_posix_thread()
            .unwrap()
            .credentials()
            .aster_mac_label();
        writeln!(printer, "{}", label)?;

        Ok(printer.bytes_written())
    }
}
