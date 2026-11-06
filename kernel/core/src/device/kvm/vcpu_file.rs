// SPDX-License-Identifier: MPL-2.0

//! A single vCPU, its execution state, and the shared KVM run page.

use super::ioctl::{KVM_RUN_MMAP_SIZE, Run};
use crate::{
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        file::{
            AccessMode, FileCommon, FileLike, MappableObject, StatusFlags, file_table::FdFlags,
        },
        pseudofs::AnonInodeFs,
    },
    kvm::{run_page::KvmRunPage, vcpu::Vcpu, vm::Vm},
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
    vm::vmar::FileMmapRequest,
};

pub(super) struct VcpuFile {
    // Keeps the VM alive while this vCPU file is open.
    _vm: Arc<Vm>,
    vcpu: Arc<Vcpu>,
    run_page: KvmRunPage,
    common: FileCommon,
}

impl VcpuFile {
    pub(super) fn new(vm: Arc<Vm>, vcpu: Arc<Vcpu>) -> Result<Self> {
        Ok(Self {
            _vm: vm,
            vcpu,
            run_page: KvmRunPage::new(KVM_RUN_MMAP_SIZE)?,
            common: FileCommon::new(
                AnonInodeFs::new_path(|_| "anon_inode:kvm-vcpu".to_string()),
                AccessMode::O_RDWR,
                StatusFlags::empty(),
            ),
        })
    }

    fn run(&self) -> Result<i32> {
        self.vcpu.run(&self.run_page)?;
        Ok(0)
    }
}

impl FileLike for VcpuFile {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            Run => {
                self.run()
            }
            _ => {
                self.vcpu.ioctl(raw_ioctl)
            }
        })
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn mappable(&self, request: FileMmapRequest) -> Result<MappableObject<'_>> {
        if !request.is_shared() {
            return_errno_with_message!(Errno::EINVAL, "the run page requires a shared mapping");
        }

        Ok(MappableObject::Device(&self.run_page))
    }

    fn dump_proc_fdinfo(self: Arc<Self>, _fd_flags: FdFlags) -> Box<dyn core::fmt::Display> {
        Box::new("hypervisor_vcpu\n")
    }
}

impl Pollable for VcpuFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}
