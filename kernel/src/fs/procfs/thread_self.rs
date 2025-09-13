// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::{ProcSymBuilder, SymOps},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
};

/// Represents the inode at `/proc/self-thread`.
pub struct ThreadSelfSymOps;

impl ThreadSelfSymOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/thread_self.c#L50>
        ProcSymBuilder::new(Self, InodeMode::from_bits_truncate(0o777))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl SymOps for ThreadSelfSymOps {
    fn read_link(&self) -> Result<String> {
        let pid = current!().pid();
        let tid = current_thread!().as_posix_thread().unwrap().tid();
        Ok(format!("{}/task/{}", pid, tid))
    }
}
