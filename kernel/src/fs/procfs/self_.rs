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
        ProcSymBuilder::new(Self)
            .parent(parent)
            // Reference: <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/fs/proc/self.c#L50>
            .mode(InodeMode::from_bits_truncate(0o777))
            .build()
            .unwrap()
    }
}

impl SymOps for SelfSymOps {
    fn read_link(&self) -> Result<String> {
        Ok(current!().pid().to_string())
    }
}
