// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use device_id::{DeviceId, MajorId, MinorId};
use ostd::task::Task;

use super::{ioctl::*, vm_file::VmFile};
use crate::{
    device::{Device, DeviceType},
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags, file_table::FdFlags},
        vfs::{inode::FileOps, path::Path},
    },
    kvm::vm::Vm,
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

#[derive(Default)]
pub(super) struct KvmDevice {
    next_vm_id: Arc<AtomicU32>,
}

impl Device for KvmDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(10), MinorId::new(232))
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        Some(DevtmpfsNodeMeta::new("kvm").unwrap())
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(KvmDeviceFile {
            next_vm_id: self.next_vm_id.clone(),
        }))
    }
}

struct KvmDeviceFile {
    next_vm_id: Arc<AtomicU32>,
}

impl KvmDeviceFile {
    fn alloc_vm_id(&self) -> u32 {
        self.next_vm_id.fetch_add(1, Ordering::Relaxed)
    }
}

impl PerOpenFileOps for KvmDeviceFile {
    fn ioctl(&self, _path: &Path, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            GetApiVersion => {
                Ok(KVM_API_VERSION)
            }
            CreateVm => {
                if raw_ioctl.arg() != 0 {
                    return_errno_with_message!(Errno::EINVAL, "unsupported VM type");
                }

                let vm_id = self.alloc_vm_id();
                let vm = Vm::new(vm_id)?;
                let vm_file = Arc::new(VmFile::new(vm)?);

                // Insert into the current process's file table
                let current = Task::current().unwrap();
                let file_table = current.as_thread_local().unwrap().borrow_file_table();
                let mut file_table_locked = file_table.unwrap().write();
                let vm_fd = file_table_locked.insert(vm_file, FdFlags::empty());

                Ok(vm_fd.into())
            }
            CheckExtension => {
                Ok(check_extension(raw_ioctl.arg()))
            }
            GetVcpuMmapSize => {
                Ok(KVM_RUN_MMAP_SIZE as i32)
            }
            _ => {
                return_errno_with_message!(Errno::ENOTTY, "unsupported KVM ioctl")
            }
        })
    }

    fn check_seekable(&self) -> Result<()> {
        return_errno!(Errno::ESPIPE)
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}

impl FileOps for KvmDeviceFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _flags: StatusFlags,
    ) -> Result<usize> {
        return_errno!(Errno::EINVAL)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _flags: StatusFlags,
    ) -> Result<usize> {
        return_errno!(Errno::EINVAL)
    }
}

impl Pollable for KvmDeviceFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        (IoEvents::IN | IoEvents::OUT) & mask
    }
}
