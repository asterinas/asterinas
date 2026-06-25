// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::posix_thread::{AsPosixThread, MAX_THREAD_NAME_LEN},
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/comm` (and also `/proc/[pid]/comm`).
pub struct CommFileOps(TidDirOps);

impl CommFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3336>
        ProcFile::new(Self(dir.clone()), parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for CommFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let Some(thread) = self.0.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };

        let Some(posix_thread) = thread.as_posix_thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        let mut comm = posix_thread.thread_name().lock().name().to_bytes().to_vec();
        comm.push(b'\n');

        let mut vm_reader = VmReader::from(&comm[offset.min(comm.len())..]);
        let bytes_read = writer.write_fallible(&mut vm_reader)?;

        Ok(bytes_read)
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let write_len = reader.remain();

        let Some((thread, process)) = self.0.thread_and_process() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };

        let Some(current_task) = Task::current() else {
            return_errno_with_message!(Errno::ESRCH, "the current thread does not exist");
        };
        let Some(current_posix_thread) = current_task.as_posix_thread() else {
            return_errno_with_message!(Errno::ESRCH, "the current thread does not exist");
        };
        let current_process = current_posix_thread.process();
        if !Arc::ptr_eq(&current_process, &process) {
            return_errno_with_message!(Errno::EINVAL, "the thread group is different");
        }

        let mut name_buf = [0; MAX_THREAD_NAME_LEN - 1];
        let read_len = write_len.min(name_buf.len());
        reader.read_fallible(&mut VmWriter::from(&mut name_buf[..read_len]))?;

        let Some(posix_thread) = thread.as_posix_thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        posix_thread
            .thread_name()
            .lock()
            .set_name_from_bytes(&name_buf[..read_len]);

        Ok(write_len)
    }
}
