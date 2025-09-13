// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    process::Process,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/comm` (and also `/proc/[pid]/comm`).
pub struct CommFileOps(Arc<Process>);

impl CommFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3336>
        ProcFileBuilder::new(Self(process_ref), InodeMode::from_bits_truncate(0o644))
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
