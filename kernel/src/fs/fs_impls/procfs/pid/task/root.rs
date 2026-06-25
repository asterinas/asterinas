// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcSym, ProcSymOps},
        vfs::inode::{Inode, SymbolicLink},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/root` (and also `/proc/[pid]/root`).
pub struct RootSymOps(TidDirOps);

impl RootSymOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3313>
        ProcSym::new(Self(dir.clone()), parent, mkmod!(a+rwx))
    }
}

impl ProcSymOps for RootSymOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        let Some(thread) = self.0.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };

        let Some(posix_thread) = thread.as_posix_thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        let root = {
            let fs = posix_thread.read_fs();
            let resolver = fs.resolver().read();
            resolver.root().clone()
        };

        Ok(SymbolicLink::Path(root))
    }
}
