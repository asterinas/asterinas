// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/mountinfo` (and also `/proc/[pid]/mountinfo`).
pub struct MountInfoFileOps {
    thread_ref: Arc<Thread>,
}

impl MountInfoFileOps {
    pub fn new_inode(thread_ref: Arc<Thread>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3352>
        ProcFileBuilder::new(Self { thread_ref }, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for MountInfoFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        unreachable!()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let posix_thread = self.thread_ref.as_posix_thread().unwrap();
        let fs = posix_thread.read_fs();
        let fs_resolver = fs.resolver().read();
        let root_mount = fs_resolver.root().mount_node();

        root_mount.read_mount_info(offset, writer)
    }
}
