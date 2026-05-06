// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcSym, SymOps},
        vfs::inode::{Inode, SymbolicLink},
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/exe` (and also `/proc/[pid]/exe`).
pub struct ExeSymOps(TidDirOps);

impl ExeSymOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3350>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L174-L175>
        ProcSym::new(Self(dir.clone()), parent, mkmod!(a+rwx))
    }
}

impl SymOps for ExeSymOps {
    fn read_link(&self) -> Result<SymbolicLink> {
        let Some(process) = self.0.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        };

        let vmar_guard = process.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return_errno_with_message!(Errno::ENOENT, "the process has exited");
        };
        let path = vmar.process_vm().executable_file().clone();

        Ok(SymbolicLink::Path(path))
    }
}
