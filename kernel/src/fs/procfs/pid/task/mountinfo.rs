// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/mountinfo` (and also `/proc/[pid]/mountinfo`).
pub struct MountInfoFileOps(TidDirOps);

impl MountInfoFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3352>
        ProcFileBuilder::new(Self(dir.clone()), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for MountInfoFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let thread = self.0.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let fs = posix_thread.read_fs();
        let path_resolver = fs.resolver().read();

        path_resolver.read_mount_info(offset, writer)
    }
}
