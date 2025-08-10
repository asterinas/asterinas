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
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/pid.c#L721>
        ProcFileBuilder::new(Self, InodeMode::from_bits_truncate(0o644))
            .parent(parent)
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
        warn!("writing to `/proc/sys/kernel/pid_max` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/sys/kernel/pid_max` is not supported"
        );
    }
}
