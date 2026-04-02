// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use aster_util::printer::VmPrinter;

use super::{TidDirOps, process_from_pid_entry};
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{FileOps, ProcFileBuilder, read_i32_from},
        vfs::inode::Inode,
    },
    prelude::*,
    process::pid_table::PidEntry,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/oom_score_adj` (and also `/proc/[pid]/oom_score_adj`).
pub struct OomScoreAdjFileOps(Arc<PidEntry>);

impl OomScoreAdjFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3386>
        ProcFileBuilder::new(Self(dir.pid_entry().clone()), mkmod!(a+r, u+w))
            .parent(parent)
            .need_revalidation()
            .build()
            .unwrap()
    }
}

impl FileOps for OomScoreAdjFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let process = process_from_pid_entry(&self.0)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the process has been reaped"))?;
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
        let process = process_from_pid_entry(&self.0)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the process has been reaped"))?;
        process.oom_score_adj().store(val as i16, Ordering::Relaxed);

        Ok(read_bytes)
    }
}

// FIXME: Support OOM killer and move these constants to a more appropriate place.
const OOM_SCORE_ADJ_MIN: i32 = -1000;
const OOM_SCORE_ADJ_MAX: i32 = 1000;
