// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Write, sync::atomic::Ordering};

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/oom_score_adj` (and also `/proc/[pid]/oom_score_adj`).
pub struct OomScoreAdjFileOps(Arc<Process>);

impl OomScoreAdjFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3386>
        ProcFileBuilder::new(Self(process_ref), InodeMode::from_bits_truncate(0o644))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for OomScoreAdjFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let oom_score_adj = self.0.oom_score_adj().load(Ordering::Relaxed);

        let mut output = String::new();
        writeln!(output, "{}", oom_score_adj).unwrap();
        Ok(output.into_bytes())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        // TODO: Extend the `ReadCString` trait to read a C string without
        // requiring the nul terminator.
        let (val, read_bytes) = read_i32_from(reader)?;
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

/// Reads an `i32` from the given `VmReader`.
///
/// Returns the read value and the number of bytes read from the given `VmReader`.
fn read_i32_from(reader: &mut VmReader) -> Result<(i32, usize)> {
    let mut buf = [0u8; BUF_SIZE_I32];

    let read_bytes = reader.read_fallible(&mut (&mut buf[..BUF_SIZE_I32 - 1]).into())?;
    let val = CStr::from_bytes_until_nul(&buf)
        .unwrap()
        .to_str()
        .ok()
        .and_then(|str| str.parse::<i32>().ok())
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the value is not a valid integer"))?;
    Ok((val, read_bytes))
}

/// Worst case buffer size needed for holding an integer.
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/internal.h#L163>.
const BUF_SIZE_I32: usize = 13;

// FIXME: Support OOM killer and move these constants to a more appropriate place.
const OOM_SCORE_ADJ_MIN: i32 = -1000;
const OOM_SCORE_ADJ_MAX: i32 = 1000;
