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
pub struct CommFileOps(Arc<Process>);

impl CommFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CommFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let mut comm_output = {
            let exe_path = self.0.executable_path();
            let last_component = exe_path.rsplit('/').next().unwrap_or(&exe_path);
            let mut comm = last_component.as_bytes().to_vec();
            comm.push(b'\0');
            comm.truncate(TASK_COMM_LEN);
            comm
        };
        comm_output.push(b'\n');
        Ok(comm_output)
    }
}

const TASK_COMM_LEN: usize = 16;
