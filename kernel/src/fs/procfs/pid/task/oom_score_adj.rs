// SPDX-License-Identifier: MPL-2.0

use core::{ffi::CStr, fmt::Write, sync::atomic::Ordering};

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/oom_score_adj` (and also `/proc/[pid]/oom_score_adj`).
pub struct OOMScoreAdjFileOps(Arc<Process>);

impl OOMScoreAdjFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref))
            .parent(parent)
            // Reference: <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/fs/proc/base.c#L3380>
            .mode(InodeMode::from_bits_truncate(0o644))
            .build()
            .unwrap()
    }
}

impl FileOps for OOMScoreAdjFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let mut output = String::new();
        let oom_score_adj = self.0.oom_score_adj().load(Ordering::Relaxed);
        writeln!(output, "{}", oom_score_adj).unwrap();
        Ok(output.into_bytes())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let mut buf = [0u8; BUF_SIZE_I32];
        let written_bytes = reader.read_fallible(&mut (&mut buf[..BUF_SIZE_I32 - 1]).into())?;
        buf[written_bytes] = 0; // Null-terminate the buffer.

        let str = CStr::from_bytes_until_nul(&buf[0..written_bytes + 1])?
            .to_str()?
            .trim();
        let val = str.parse::<i32>().map_err(|_| Error::new(Errno::EINVAL))?;
        if !(OOM_SCORE_ADJ_MIN..=OOM_SCORE_ADJ_MAX).contains(&val) {
            return_errno_with_message!(Errno::EINVAL, "`oom_score_adj` out of range");
        }

        self.0.oom_score_adj().store(val as i16, Ordering::Relaxed);

        Ok(written_bytes)
    }
}

/// Worst case buffer size needed for holding an integer
const BUF_SIZE_I32: usize = 13;

// FIXME: Support OOM killer and move these constants to a more appropriate place.
const OOM_SCORE_ADJ_MIN: i32 = -1000;
const OOM_SCORE_ADJ_MAX: i32 = 1000;
