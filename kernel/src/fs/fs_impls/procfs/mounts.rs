// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{ProcSymBuilder, SymOps},
        utils::{Inode, SymbolicLink, mkmod},
    },
    prelude::*,
};

/// Represents the inode at `/proc/mounts`.
pub struct MountsSymOps;

impl MountsSymOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/root.c#L291>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L466>
        ProcSymBuilder::new(Self, mkmod!(a+rwx))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl SymOps for MountsSymOps {
    fn read_link(&self) -> Result<SymbolicLink> {
        Ok(SymbolicLink::Plain("self/mounts".to_string()))
    }
}
