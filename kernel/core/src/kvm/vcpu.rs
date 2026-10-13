
use super::{
    vm::Vm,
    arch::vcpu::*,
    run_page::KvmRun,
};
use crate::prelude::*;
use crate::util::ioctl::RawIoctl;

pub struct Vcpu {
    id: u32,
    pub vm: Weak<Vm>,
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

    pub fn init(&self) {
        self.arch.init();
    }

    pub fn vm(&self) -> Result<Arc<Vm>> {
        self.vm
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "vm not found"))
    }

    pub fn get_onereg(&self, reg_id: usize) -> usize {
        self.arch.get_onereg(reg_id)
    }

    pub fn set_onereg(&self, reg_id: usize, reg_val: usize) {
        self.arch.set_onereg(reg_id, reg_val);
    }

    pub fn run(self: &Arc<Self>, run: Arc<KvmRun>) -> Result<()> {
        let _run_guard = self.run_lock.lock();
        let page_table_phys = self.vm()?.memory().guest_mem().root_paddr();

        self.arch.run(run, page_table_phys)
    }

    pub fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        // Architecture-independent ioctls are already handled in `vcpu_file.rs`
        // Here, we need to handle architecture-specific ioctls.
        self.arch.ioctl(raw_ioctl)
    }
}

impl Drop for Vcpu {
    fn drop(&mut self) {
        debug!("hypervisor: release VCPU {}.", self.id);
    }
}
