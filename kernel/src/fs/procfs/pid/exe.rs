// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{ProcSymBuilder, SymOps},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    Process,
};

/// Represents the inode at `/proc/[pid]/exe`.
pub struct ExeSymOps(Arc<Process>);

impl ExeSymOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSymBuilder::new(Self(process_ref))
            .parent(parent)
            // Reference:
            // <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/fs/proc/base.c#L174-L175>
            // <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/fs/proc/base.c#L3344>
            .mode(InodeMode::from_bits_truncate(0o777))
            .build()
            .unwrap()
    }
}

impl SymOps for ExeSymOps {
    fn read_link(&self) -> Result<String> {
        Ok(self.0.executable_path())
    }
}
