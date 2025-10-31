// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/comm` (and also `/proc/[pid]/comm`).
pub struct CommFileOps(Arc<Process>);

impl CommFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.process_ref.clone();
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3336>
        ProcFileBuilder::new(Self(process_ref), mkmod!(a+r, u+w))
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

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/[pid]/comm` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/[pid]/comm` is not supported"
        );
    }
}

const TASK_COMM_LEN: usize = 16;
