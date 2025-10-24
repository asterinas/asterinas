// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        fs_resolver::FsItem,
        procfs::{ProcSymBuilder, SymOps},
        utils::{mkmod, Inode, ReadLinkResult},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/exe` (and also `/proc/[pid]/exe`).
pub struct ExeSymOps(Arc<Process>);

impl ExeSymOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
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
    fn read_link(&self) -> Result<ReadLinkResult> {
        let res = match self.0.executable_fsitem() {
            FsItem::Real(path) => ReadLinkResult::Real(path.abs_path()),
            FsItem::Pseudo(pseudo_file) => ReadLinkResult::Pseudo(pseudo_file.clone()),
        };
        Ok(res)
    }
}
