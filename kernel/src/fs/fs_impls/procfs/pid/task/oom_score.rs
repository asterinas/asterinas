// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use aster_util::printer::VmPrinter;

use super::{TidDirOps, oom_score_adj::OOM_SCORE_ADJ_MAX};
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/oom_score` (and also `/proc/[pid]/oom_score`).
pub struct OomScoreFileOps(TidDirOps);

impl OomScoreFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3390>
        ProcFile::new(Self(dir.clone()), parent, mkmod!(a+r))
    }
}

impl ProcFileOps for OomScoreFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let Some(process) = self.0.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        };

        // Asterinas does not have OOM killer badness accounting yet, so expose the supported
        // adjustment component as the process's current killability score.
        let oom_score_adj = process.oom_score_adj().load(Ordering::Relaxed) as i32;
        let oom_score = oom_score_adj.clamp(0, OOM_SCORE_ADJ_MAX);
        writeln!(printer, "{}", oom_score)?;

        Ok(printer.bytes_written())
    }
}
