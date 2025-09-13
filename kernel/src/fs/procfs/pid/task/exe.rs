// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{ProcSymBuilder, SymOps},
        utils::{Inode, InodeMode},
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
        ProcSymBuilder::new(Self(process_ref), InodeMode::from_bits_truncate(0o777))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl SymOps for ExeSymOps {
    fn read_link(&self) -> Result<String> {
        Ok(self.0.executable_path())
    }
}
