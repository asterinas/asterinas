// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{ProcSymBuilder, SymOps},
        utils::Inode,
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
            .build()
            .unwrap()
    }
}

impl SymOps for ExeSymOps {
    fn read_link(&self) -> Result<String> {
        Ok(self.0.executable_path())
    }
}
