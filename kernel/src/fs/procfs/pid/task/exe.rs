// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        fs_resolver::PathOrInode,
        procfs::{ProcSymBuilder, SymOps},
        utils::{mkmod, Inode, SymbolicLink},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/exe` (and also `/proc/[pid]/exe`).
pub struct ExeSymOps(Arc<Process>);

impl ExeSymOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.process_ref.clone();
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3350>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L174-L175>
        ProcSymBuilder::new(Self(process_ref), mkmod!(a+rwx))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl SymOps for ExeSymOps {
    fn read_link(&self) -> Result<SymbolicLink> {
        let vmar_guard = self.0.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return_errno_with_message!(Errno::ENOENT, "the process has exited");
        };

        let res = match vmar.process_vm().executable_file().clone() {
            PathOrInode::Path(path) => SymbolicLink::Path(path),
            PathOrInode::Inode(inode) => SymbolicLink::Inode(inode),
        };
        Ok(res)
    }
}
