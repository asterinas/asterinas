// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    Process,
};

/// Represents the inode at `/proc/[pid]/comm`.
pub struct EnvironFileOps(Arc<Process>);

impl EnvironFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for EnvironFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let envp = self.0.envp()?;
        let mut environ_output = Vec::new();
        for var in envp {
            environ_output.extend_from_slice(var.as_bytes());
            environ_output.push(b'\0');
        }
        Ok(environ_output)
    }
}
