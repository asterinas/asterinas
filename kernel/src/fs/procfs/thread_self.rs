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
        ProcSymBuilder::new(Self)
            .parent(parent)
            // Reference: <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/fs/proc/thread_self.c#L50>
            .mode(InodeMode::from_bits_truncate(0o777))
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
