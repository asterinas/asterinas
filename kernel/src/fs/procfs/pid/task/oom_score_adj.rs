// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/oom_score_adj` (and also `/proc/[pid]/oom_score_adj`).
pub struct OomScoreAdjFileOps(Arc<Process>);

impl OomScoreAdjFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.process_ref.clone();
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3386>
        ProcFileBuilder::new(Self(process_ref), mkmod!(a+r, u+w))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for OomScoreAdjFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let oom_score_adj = self.0.oom_score_adj().load(Ordering::Relaxed);
        writeln!(printer, "{}", oom_score_adj)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (cstr, read_bytes) = reader.read_cstring_until_end(BUF_SIZE_I32 - 1)?;
        let val = cstr
            .to_str()
            .ok()
            .map(|str| str.trim())
            .and_then(|str| str.parse::<i32>().ok())
            .ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "the value is not a valid integer")
            })?;
        if !(OOM_SCORE_ADJ_MIN..=OOM_SCORE_ADJ_MAX).contains(&val) {
            return_errno_with_message!(Errno::EINVAL, "the OOM score adjustment is out of range");
        }

        // TODO: If the new adjustment value is smaller than the smallest
        // adjustment value that the process has ever reached and the writer
        // does not have the `SYS_RESOURCE` capability, we should fail with
        // `EACCES`. See
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L1152>.
        self.0.oom_score_adj().store(val as i16, Ordering::Relaxed);

        Ok(read_bytes)
    }
}

/// Worst case buffer size needed for holding an integer.
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/internal.h#L163>.
const BUF_SIZE_I32: usize = 13;

// FIXME: Support OOM killer and move these constants to a more appropriate place.
const OOM_SCORE_ADJ_MIN: i32 = -1000;
const OOM_SCORE_ADJ_MAX: i32 = 1000;
