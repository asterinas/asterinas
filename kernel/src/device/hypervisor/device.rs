// SPDX-License-Identifier: MPL-2.0

//! The KVM-compatible hypervisor device.

use device_id::{DeviceId, MajorId, MinorId};
use ostd::task::Task;

use super::{
    KVM_MAJOR, KVM_MINOR,
    ioctl::{CreateVm, GetApiVersion, KVM_API_VERSION},
    vm::{Vm, VmFile},
};
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

pub(super) struct HypervisorDevice;

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
        Ok(Box::new(HypervisorDeviceFile))
    }
}

struct HypervisorDeviceFile;

impl Pollable for HypervisorDeviceFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        (IoEvents::IN | IoEvents::OUT) & mask
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
                if raw_ioctl.arg() != 0 {
                    return_errno_with_message!(Errno::EINVAL, "unsupported KVM machine type");
                }

                let vm_file = Arc::new(VmFile::new(Vm::new()));
                let current = Task::current().unwrap();
                let mut file_table = current.as_thread_local().unwrap().borrow_file_table_mut();
                let vm_fd = file_table
                    .unwrap()
                    .write()
                    .insert(vm_file, FdFlags::empty());
                Ok(vm_fd.into())
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "unknown KVM device ioctl command"),
        })
    }

    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the device is not seekable");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
