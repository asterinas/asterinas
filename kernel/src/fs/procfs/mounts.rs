// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{ProcSymBuilder, SymOps},
        utils::{Inode, SymbolicLink, mkmod},
    },
    prelude::*,
};

/// Represents the inode at `/proc/mounts`.
/// This is a symbolic link to `self/mounts`.
pub struct MountsSymOps;

impl MountsSymOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
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
