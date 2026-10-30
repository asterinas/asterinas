// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::vm::GuestContext,
    vm::{GuestMode, GuestModeHooks, GuestReturnReason},
};

use crate::{
    device::hypervisor::ioctl,
    kvm::{arch::vcpu::PendingIo, run_page::KvmRunPage, vm::Vm},
    prelude::*,
};

pub(crate) struct Vcpu {
    _id: u32,
    vm: Weak<Vm>,
    state: Mutex<VcpuState>,
}

pub(super) struct VcpuState {
    pub guest_context: GuestContext,
    pub pending_io: Option<PendingIo>,
}

impl Vcpu {
    pub(super) fn new(id: u32, vm: &Arc<Vm>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            _id: id,
            vm: Arc::downgrade(vm),
            state: Mutex::new(VcpuState {
                guest_context: Self::new_guest_context()?,
                pending_io: None,
            }),
        }))
    }

    pub(super) fn state(&self) -> MutexGuard<'_, VcpuState> {
        self.state.lock()
    }

    pub(super) fn vm(&self) -> Arc<Vm> {
        self.vm.upgrade().unwrap()
    }

    pub(crate) fn run(&self, run_page: &KvmRunPage) -> Result<()> {
        let mut state = self.state();

        self.finish_pending(&mut state);

        // Handle immediate exit.
        if run_page.read_run_val::<u8>(ioctl::KVM_RUN_IMMEDIATE_EXIT_OFFSET)? != 0 {
            return_errno!(Errno::EINTR);
        }

        let guest_mode = GuestMode::new()?;
        let vm = self.vm();
        loop {
            if self.has_kernel_event() {
                return_errno!(Errno::EINTR);
            }

            let run_result =
                match guest_mode.execute(&mut state.guest_context, vm.memory().guest_mem(), self) {
                    Ok(exit_info) => exit_info,
                    Err(err) => {
                        error!("hypervisor: GuestMode::execute failed: {:?}", err);
                        return Err(err.into());
                    }
                };

            let exit_info = match run_result {
                GuestReturnReason::VmExit(exit_info) => exit_info,
                GuestReturnReason::KernelEvent => {
                    continue;
                }
            };

            if self.handle_exit(&mut state, exit_info, run_page)? {
                return Ok(());
            }
        }
    }
}
