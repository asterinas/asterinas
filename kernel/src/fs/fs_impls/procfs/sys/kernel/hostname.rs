// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/sys/kernel/hostname`.
pub struct HostnameFileOps;

impl HostnameFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/utsname_sysctl.c>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for HostnameFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let Some(current_task) = Task::current() else {
            return_errno_with_message!(Errno::ESRCH, "the current thread does not exist");
        };
        let Some(thread_local) = current_task.as_thread_local() else {
            return_errno_with_message!(Errno::ESRCH, "the current thread does not exist");
        };
        let ns_proxy = thread_local.borrow_ns_proxy();
        let uts_name = ns_proxy.unwrap().uts_ns().uts_name();

        let mut hostname = uts_name.hostname().to_bytes().to_vec();
        hostname.push(b'\n');

        let mut vm_reader = VmReader::from(&hostname[offset.min(hostname.len())..]);
        let bytes_read = writer.write_fallible(&mut vm_reader)?;

        Ok(bytes_read)
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/kernel/hostname` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/sys/kernel/hostname` is not supported"
        );
    }
}
