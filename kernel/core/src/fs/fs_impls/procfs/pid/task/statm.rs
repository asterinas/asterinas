// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    thread::Thread,
    vm::vmar::RssType,
};

/// Represents the inode at `/proc/[pid]/statm` (and also `/proc/[pid]/task/[tid]/statm`).
///
/// All fields are measured in pages. Threads of a process share the same address
/// space, so the values are process-wide.
///
/// Fields: `size resident shared text lib data dt`.
/// See <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/array.c#L683>.
pub struct StatmFileOps(TidDirOps);

impl StatmFileOps {
    pub fn new_thread_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        Self::new_inode(dir.clone(), parent)
    }

    fn new_inode(dir: TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3342>
        ProcFile::new(Self(dir), parent, mkmod!(a+r))
    }
}

impl ProcFileOps for StatmFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let Some(process) = self.0.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        };

        let (size, resident, shared) = if let Some(vmar_ref) = process.lock_vmar().as_ref() {
            let size = vmar_ref.get_mappings_total_size() / PAGE_SIZE;
            let anon = vmar_ref.get_rss_counter(RssType::Anon);
            let file = vmar_ref.get_rss_counter(RssType::File);
            (size, anon + file, file)
        } else {
            (0, 0, 0)
        };

        // The `text`, `lib`, `data`, and `dt` fields are reported as `0`: `lib` and
        // `dt` are always `0` on modern Linux, and Asterinas does not track the code
        // and data segment page counts for this file yet.
        writeln!(printer, "{} {} {} 0 0 0 0", size, resident, shared)?;

        Ok(printer.bytes_written())
    }
}
