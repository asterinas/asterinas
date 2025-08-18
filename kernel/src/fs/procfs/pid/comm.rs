// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{
            pid::util::PidOrTid,
            template::{FileOps, ProcFileBuilder},
        },
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/comm` or `/proc/[pid]/task/[tid]/comm`.
pub struct CommFileOps(PidOrTid);

impl CommFileOps {
    pub fn new_inode(pid_or_tid: PidOrTid, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(pid_or_tid))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CommFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let mut comm_output = {
            let exe_path = self.0.process().executable_path();
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
