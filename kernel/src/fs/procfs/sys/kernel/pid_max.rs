// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    process::posix_thread::PID_MAX,
};

/// Represents the inode at `/proc/sys/kernel/pid_max`.
pub struct PidMaxFileOps;

impl PidMaxFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self)
            .parent(parent)
            // Reference: <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/kernel/pid.c#L725>
            .mode(InodeMode::from_bits_truncate(0o644))
            .build()
            .unwrap()
    }
}

impl FileOps for PidMaxFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let output = format!("{}\n", PID_MAX);
        Ok(output.into_bytes())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("Setting `PID_MAX` is not supported currently.");
        Err(Error::new(Errno::EOPNOTSUPP))
    }
}
