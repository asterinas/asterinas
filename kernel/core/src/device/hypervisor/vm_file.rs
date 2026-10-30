// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use super::{ioctl::*, vcpu_file::VcpuFile};
use crate::{
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        file::{AccessMode, FileCommon, FileLike, StatusFlags, file_table::FdFlags},
        pseudofs::AnonInodeFs,
    },
    kvm::vm::Vm,
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

/// A file description that owns a VM.
pub(super) struct VmFile {
    vm: Arc<Vm>,
    common: FileCommon,
}

impl VmFile {
    pub(super) fn new(vm: Arc<Vm>) -> Result<Self> {
        Ok(Self {
            vm,
            common: FileCommon::new(
                AnonInodeFs::new_path(|_| "anon_inode:kvm-vm".to_string()),
                AccessMode::O_RDWR,
                StatusFlags::empty(),
            ),
        })
    }
}

impl FileLike for VmFile {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            CheckExtension => {
                Ok(check_extension(raw_ioctl.arg()))
            }
            CreateVcpu => {
                let vcpu_id = u32::try_from(raw_ioctl.arg())?;
                let vcpu = self.vm.create_vcpu(vcpu_id)?;
                let vcpu_file = Arc::new(VcpuFile::new(self.vm.clone(), vcpu)?);

                // Insert into the current process's file table
                let current = Task::current().unwrap();
                let file_table = current.as_thread_local().unwrap().borrow_file_table();
                let mut file_table_locked = file_table.unwrap().write();
                let vcpu_fd = file_table_locked.insert(vcpu_file, FdFlags::empty());

                Ok(vcpu_fd.into())
            }
            cmd @ SetUserMemoryRegion => {
                let region: UserMemoryRegion = cmd.read()?;
                if region.flags != 0 {
                    return_errno_with_message!(Errno::EINVAL, "unsupported memory region flags");
                }

                self.vm.memory().set_region(
                    region.slot,
                    region.guest_phys_addr as _,
                    region.memory_size as _,
                    region.userspace_addr as _,
                )?;
                Ok(0)
            }
            _ => {
                self.vm.ioctl(raw_ioctl)
            }
        })
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn dump_proc_fdinfo(self: Arc<Self>, _fd_flags: FdFlags) -> Box<dyn core::fmt::Display> {
        Box::new("hypervisor_vm\n")
    }
}

impl Pollable for VmFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}
