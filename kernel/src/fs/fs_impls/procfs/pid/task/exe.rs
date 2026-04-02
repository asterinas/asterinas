// SPDX-License-Identifier: MPL-2.0

use super::{TidDirOps, process_from_pid_entry};
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcSymBuilder, SymOps},
        vfs::inode::{Inode, SymbolicLink},
    },
    prelude::*,
    process::pid_table::PidEntry,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/exe` (and also `/proc/[pid]/exe`).
pub struct ExeSymOps(Arc<PidEntry>);

impl ExeSymOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3350>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L174-L175>
        ProcSymBuilder::new(Self(dir.pid_entry().clone()), mkmod!(a+rwx))
            .parent(parent)
            .need_revalidation()
            .build()
            .unwrap()
    }
}

impl SymOps for ExeSymOps {
    fn read_link(&self) -> Result<SymbolicLink> {
        let process = process_from_pid_entry(&self.0)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the process has been reaped"))?;
        let vmar_guard = process.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return_errno_with_message!(Errno::ESRCH, "the process has been reaped");
        };
        let path = vmar.process_vm().executable_file().clone();

        Ok(SymbolicLink::Path(path))
    }
}
