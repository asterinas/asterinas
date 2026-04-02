// SPDX-License-Identifier: MPL-2.0

use super::{TidDirOps, process_from_pid_entry};
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{FileOps, ProcFileBuilder},
        vfs::inode::Inode,
    },
    prelude::*,
    process::pid_table::PidEntry,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/comm` (and also `/proc/[pid]/comm`).
pub struct CommFileOps(Arc<PidEntry>);

impl CommFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3336>
        ProcFileBuilder::new(Self(dir.pid_entry().clone()), mkmod!(a+r, u+w))
            .parent(parent)
            .need_revalidation()
            .build()
            .unwrap()
    }
}

impl FileOps for CommFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let Some(process) = process_from_pid_entry(&self.0) else {
            return_errno_with_message!(Errno::ESRCH, "the process has been reaped");
        };
        let vmar_guard = process.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            // According to Linux behavior, return an empty string
            // if the process is a zombie process.
            return Ok(0);
        };

        let executable_file_name = vmar.process_vm().executable_file().name();
        let mut comm = executable_file_name.as_bytes().to_vec();
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
