// SPDX-License-Identifier: MPL-2.0

//! VM file descriptor implementation

use ostd::task::Task;

use super::vcpu_file::VcpuFile;
use crate::{
    fs::{
        file::{
            AccessMode, FileCommon, FileLike, StatusFlags,
            file_table::FdFlags,
        },
        pseudofs::AnonInodeFs,
    },
    prelude::*,
    util::ioctl::{RawIoctl},
    kvm::vm::Vm,
    kvm::ioctl::*,
    dispatch_ioctl
};

use super::ioctl::*;

/// VM file descriptor
pub(super) struct VmFile {
    /// VmFile owns the Vm instance, but why 'Arc'?
    /// VcpuFiles need to reference the Vm, but can't act like
    /// struct VcpuFile<'a> { vm: &'a Vm, ... } because the
    /// VcpuFile needs to be 'static to be stored in the file table.
    vm: Arc<Vm>,
    common: FileCommon,
}

impl VmFile {
    /// Creates a new VM file
    pub(super) fn new(vm: Arc<Vm>) -> Self {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[kvm-vm]".to_string());
        Self {
            vm,
            common: FileCommon::new(pseudo_path, AccessMode::O_RDWR, StatusFlags::empty()),
        }
    }
}

impl FileLike for VmFile {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read from VM file");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write to VM file");
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            CheckExtension => {
                Ok(check_extension(raw_ioctl))
            }
            CreateVcpu => {
                let vcpu_id = read_vcpu_id(raw_ioctl)?;

                // Create a file descriptor for the VCPU
                let vcpu_file = Arc::new(VcpuFile::new(self.vm.clone(), vcpu_id)?);

                // Insert into the current process's file table
                let current = Task::current().unwrap();
                let mut file_table = current.as_thread_local().unwrap().borrow_file_table_mut();
                let mut file_table_locked = file_table.unwrap().write();
                let vcpu_fd = file_table_locked.insert(vcpu_file, FdFlags::empty());

                Ok(vcpu_fd.into())
            }
            cmd @ SetUserMemoryRegion => {
                let region: UserMemoryRegion = cmd.read()?;
                self.vm.set_user_memory_region(region)?;
                Ok(0)
            }
            _ => {
                let ioctl_nr = raw_ioctl.cmd() & 0xff;
                error!(
                    "kvm: unimplemented VM ioctl command: cmd={:#x}, nr={:#x}",
                    raw_ioctl.cmd(),
                    ioctl_nr
                );
                return_errno_with_message!(Errno::ENOTTY, "unknown VM ioctl command");
            }
        })
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn dump_proc_fdinfo(self: Arc<Self>, _fd_flags: FdFlags) -> Box<dyn core::fmt::Display> {
        Box::new(alloc::format!("vm_id: {}\n", self.vm.id))
    }
}

impl crate::process::signal::Pollable for VmFile {
    fn poll(
        &self,
        _mask: crate::events::IoEvents,
        _poller: Option<&mut crate::process::signal::PollHandle>,
    ) -> crate::events::IoEvents {
        // VMs don't support polling
        crate::events::IoEvents::empty()
    }
}
