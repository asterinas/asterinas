// SPDX-License-Identifier: MPL-2.0

use super::{
    vm::Vm,
    arch::vcpu::VcpuArch,
    run_page::KvmRun,
};
use crate::prelude::*;
use crate::util::ioctl::RawIoctl;

pub struct Vcpu {
    id: u32,
    vm: Weak<Vm>,
    run_lock: Mutex<()>,
    arch: VcpuArch,
}

impl Vcpu {
    pub fn new(id: u32, vm: &Arc<Vm>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            id,
            vm: Arc::downgrade(vm),
            run_lock: Mutex::new(()),
            arch: VcpuArch::new(),
        }))
    }

    pub fn init(&self) -> Result<()> {
        self.arch.init()
    }

    pub fn vm(&self) -> Result<Arc<Vm>> {
        self.vm
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "vm not found"))
    }

    pub fn get_onereg(&self, reg_id: usize) -> Result<usize> {
        self.arch.get_onereg(reg_id)
    }

    pub fn set_onereg(&self, reg_id: usize, reg_val: usize) -> Result<()> {
        self.arch.set_onereg(reg_id, reg_val)
    }

    pub fn run(self: &Arc<Self>, run: Arc<KvmRun>) -> Result<()> {
        let _run_guard = self.run_lock.lock();

        self.arch.run(run, self.vm()?.memory().guest_mem())
    }

    pub fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        // Architecture-independent ioctls are already handled in `vcpu_file.rs`
        // Here, we need to handle architecture-specific ioctls.
        self.arch.ioctl(raw_ioctl)
    }
}

impl Drop for Vcpu {
    fn drop(&mut self) {
        debug!("kvm: release VCPU {}.", self.id);
    }
}
