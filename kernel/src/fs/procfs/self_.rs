// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{ProcSymBuilder, SymOps},
        utils::{Inode, InodeMode},
    },
    prelude::*,
};

/// Represents the inode at `/proc/self`.
pub struct SelfSymOps;

impl SelfSymOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/self.c#L50>
        ProcSymBuilder::new(Self, InodeMode::from_bits_truncate(0o777))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl SymOps for SelfSymOps {
    fn read_link(&self) -> Result<String> {
        Ok(current!().pid().to_string())
    }
}
