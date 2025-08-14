// SPDX-License-Identifier: MPL-2.0

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
}

impl FileOps for MemFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let guard = self.0.lock_root_vmar();
        let vmar = guard.as_ref().ok_or(Error::new(Errno::ENOENT))?;

        let mut reader = vmar.vm_space().reader(offset, writer.avail())?;
        let len = writer.write_fallible(&mut reader)?;
        Ok(len)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let guard = self.0.lock_root_vmar();
        let vmar = guard.as_ref().ok_or(Error::new(Errno::ENOENT))?;

        let mut writer = vmar.vm_space().writer(offset, reader.remain())?;
        let len = reader.read_fallible(&mut writer)?;
        Ok(len)
    }
}
