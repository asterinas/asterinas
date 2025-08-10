// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
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
            // Reference: <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/fs/proc/base.c#L3330>
            .mode(InodeMode::from_bits_truncate(0o644))
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
        warn!("Writing to `/proc/[pid]/comm` is not supported currently.");
        Err(Error::new(Errno::EOPNOTSUPP))
    }
}

const TASK_COMM_LEN: usize = 16;
