// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{FileOps, ProcFile, read_i32_from},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/oom_score_adj` (and also `/proc/[pid]/oom_score_adj`).
pub struct OomScoreAdjFileOps(TidDirOps);

impl OomScoreAdjFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3386>
        ProcFile::new(Self(dir.clone()), parent, mkmod!(a+r, u+w))
    }
}

impl FileOps for OomScoreAdjFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let Some(process) = self.0.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        };
        let oom_score_adj = process.oom_score_adj().load(Ordering::Relaxed);
        writeln!(printer, "{}", oom_score_adj)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (val, read_bytes) = read_i32_from(reader)?;

        if !(OOM_SCORE_ADJ_MIN..=OOM_SCORE_ADJ_MAX).contains(&val) {
            return_errno_with_message!(Errno::EINVAL, "the OOM score adjustment is out of range");
        }

        // TODO: If the new adjustment value is smaller than the smallest
        // adjustment value that the process has ever reached and the writer
        // does not have the `SYS_RESOURCE` capability, we should fail with
        // `EACCES`. See
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L1152>.
        let Some(process) = self.0.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        };
        process.oom_score_adj().store(val as i16, Ordering::Relaxed);

        Ok(read_bytes)
    }
}

// FIXME: Support OOM killer and move these constants to a more appropriate place.
const OOM_SCORE_ADJ_MIN: i32 = -1000;
const OOM_SCORE_ADJ_MAX: i32 = 1000;
