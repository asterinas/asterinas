// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::{ProcSymBuilder, SymOps},
        utils::Inode,
    },
    prelude::*,
    process::{posix_thread::AsPosixThread, ProcessState},
};

/// Represents the inode at `/proc/self-thread`.
pub struct ThreadSelfSymOps;

impl ThreadSelfSymOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSymBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl SymOps for ThreadSelfSymOps {
    fn read_link(&self) -> Result<String> {
        let pid = ProcessState::with_current_task(|process_state| process_state.pid()).unwrap();

        let tid = current_thread!().as_posix_thread().unwrap().tid();
        Ok(format!("{}/task/{}", pid, tid))
    }
}
