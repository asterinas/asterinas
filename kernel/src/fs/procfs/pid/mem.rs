// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use ostd::{
    mm::{io_util::HasVmReaderWriter, UFrame},
    task::disable_preempt,
};

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    Process,
};

/// Represents the inode at either `/proc/[pid]/mem` or `/proc/[pid]/task/[tid]/mem`.
pub struct MemFileOps(Arc<Process>);

impl MemFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref))
            .parent(parent)
            // Reference: <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/fs/proc/base.c#L3341>
            .mode(InodeMode::from_bits_truncate(0o600))
            .build()
            .unwrap()
    }

    fn access_at<F>(&self, offset: usize, len: usize, mut op: F) -> Result<usize>
    where
        F: FnMut(UFrame, usize) -> Result<()>,
    {
        let range = offset.align_down(PAGE_SIZE)..(offset + len).align_up(PAGE_SIZE);

        let vmar_guard = self.0.lock_root_vmar();
        let vmar = vmar_guard.as_ref().ok_or(Error::new(Errno::ENOENT))?;
        let preempt_guard = disable_preempt();
        let mut cursor = vmar.vm_space().cursor(&preempt_guard, &range)?;
        let mut current_va = range.start;

        while current_va < range.end {
            cursor.jump(current_va)?;
            let (_, Some((frame, _))) = cursor.query()? else {
                return_errno_with_message!(Errno::EIO, "Page not accessible");
            };

            let skip_offset = if current_va == range.start {
                offset - range.start
            } else {
                0
            };

            op(frame, skip_offset)?;

            current_va += PAGE_SIZE;
        }

        Ok(len)
    }
}

impl FileOps for MemFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let read_len = writer.avail();

        self.access_at(offset, read_len, |frame, skip_offset| {
            frame.reader().skip(skip_offset).read_fallible(writer)?;
            Ok(())
        })
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let write_len = reader.remain();

        self.access_at(offset, write_len, |frame, skip_offset| {
            frame.writer().skip(skip_offset).write_fallible(reader)?;
            Ok(())
        })
    }
}
