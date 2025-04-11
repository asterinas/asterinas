// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{ProcSymBuilder, SymOps},
        utils::Inode,
    },
    prelude::*,
    process::ProcessState,
};

/// Represents the inode at `/proc/self`.
pub struct SelfSymOps;

impl SelfSymOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSymBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl SymOps for SelfSymOps {
    fn read_link(&self) -> Result<String> {
        Ok(
            ProcessState::with_current_task(|process_state| process_state.pid())
                .unwrap()
                .to_string(),
        )
    }
}
