// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
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
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let vmar_guard = self.0.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            // According to Linux behavior, return an empty string
            // if the process is a zombie process.
            return Ok(0);
        };

        let exe_path = vmar.process_vm().executable_file().abs_path();
        let last_component = exe_path.rsplit('/').next().unwrap_or(&exe_path);
        let mut comm = last_component.as_bytes().to_vec();
        comm.truncate(TASK_COMM_LEN - 1);
        comm.push(b'\n');

        let mut vm_reader = VmReader::from(&comm[offset.min(comm.len())..]);
        let bytes_read = writer.write_fallible(&mut vm_reader)?;

        Ok(bytes_read)
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
