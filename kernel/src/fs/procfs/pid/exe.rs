// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{pid::util::PidOrTid, ProcSymBuilder, SymOps},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/exe` or `/proc/[pid]/task/[tid]/exe`.
pub struct ExeSymOps(PidOrTid);

impl ExeSymOps {
    pub fn new_inode(pid_or_tid: PidOrTid, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSymBuilder::new(Self(pid_or_tid))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl SymOps for ExeSymOps {
    fn read_link(&self) -> Result<String> {
        Ok(self.0.process().executable_path())
    }
}
