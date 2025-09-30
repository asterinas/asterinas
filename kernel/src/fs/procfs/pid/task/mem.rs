// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    Process,
};

/// Represents the inode at either `/proc/[pid]/mem` or `/proc/[pid]/task/[tid]/mem`.
pub struct MemFileOps(Arc<Process>);

impl MemFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref), mkmod!(u+rw))
            .parent(parent)
            // Reference: <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/fs/proc/base.c#L3341>
            .build()
            .unwrap()
    }
}

impl FileOps for MemFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        unreachable!()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.0.vm().read_remote(offset, writer)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.0.vm().write_remote(offset, reader)
    }
}
