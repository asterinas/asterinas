// SPDX-License-Identifier: MPL-2.0

//! KVM-compatible hypervisor device implementation.

use core::sync::atomic::{AtomicU32, Ordering};

use device_id::{DeviceId, MajorId, MinorId};
use ostd::task::Task;

use super::{KVM_MAJOR, KVM_MINOR, ioctl::*, vm::Vm, vm_file::VmFile};
use crate::{
    device::{Device, DeviceType, DevtmpfsInodeMeta},
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags, file_table::FdFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

/// The main KVM-compatible hypervisor device (`/dev/kvm`).
pub struct HypervisorDevice {
    /// Next VM ID to allocate
    next_vm_id: Arc<AtomicU32>,
}

impl HypervisorDevice {
    /// Creates a new KVM-compatible hypervisor device.
    pub fn new() -> Self {
        Self {
            next_vm_id: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl Device for HypervisorDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(KVM_MAJOR), MinorId::new(u32::from(KVM_MINOR)))
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        Some(DevtmpfsInodeMeta::new("kvm"))
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(HypervisorDeviceFile {
            next_vm_id: self.next_vm_id.clone(),
        }))
    }
}

/// File handle for the KVM-compatible hypervisor device.
struct HypervisorDeviceFile {
    next_vm_id: Arc<AtomicU32>,
}

impl HypervisorDeviceFile {
    fn alloc_vm_id(&self) -> u32 {
        self.next_vm_id.fetch_add(1, Ordering::Relaxed)
    }
}

impl Pollable for HypervisorDeviceFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileOps for HypervisorDeviceFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read from KVM device");
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write to KVM device");
    }
}

impl PerOpenFileOps for HypervisorDeviceFile {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            GetApiVersion => {
                Ok(KVM_API_VERSION)
            }
            CreateVm => {
                // Allocate a new VM ID
                let vm_id = self.alloc_vm_id();

                // Create the VM
                let vm = Vm::new(vm_id);

                // Create a file descriptor for the VM
                let vm_file = Arc::new(VmFile::new(vm));

                // Insert into the current process's file table
                let current = Task::current().unwrap();
                let mut file_table = current.as_thread_local().unwrap().borrow_file_table_mut();
                let mut file_table_locked = file_table.unwrap().write();
                let vm_fd = file_table_locked.insert(vm_file, FdFlags::empty());

                Ok(vm_fd.into())
            }
            GetVcpuMmapSize => {
                Ok(KVM_RUN_MMAP_SIZE as i32)
            }
            _ => {
                let ioctl_nr = raw_ioctl.cmd() & 0xff;
                error!(
                    "rustshyper: unimplemented device ioctl command: cmd={:#x}, nr={:#x}",
                    raw_ioctl.cmd(),
                    ioctl_nr
                );
                return_errno_with_message!(Errno::ENOTTY, "unknown device ioctl command");
            }
        })
    }

    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the device is not seekable");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
