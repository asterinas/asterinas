// SPDX-License-Identifier: MPL-2.0

use x86::msr::{self, IA32_CSTAR, IA32_FMASK, IA32_KERNEL_GSBASE, IA32_LSTAR, IA32_STAR};
use x86_64::registers::control::Cr2;

use super::x86::write_cr2_raw;
use crate::{arch::cpu::context::FpuContext, irq::DisabledLocalIrqGuard};

/// Host state not restored by the VMCS host-state area.
pub(super) struct HostContext {
    fpu: FpuContext,
    cr2: u64,
    run_msrs: [u64; 5],
}

impl HostContext {
    pub(super) fn new() -> Self {
        Self {
            fpu: FpuContext::new(),
            cr2: 0,
            run_msrs: [0; 5],
        }
    }

    pub(super) fn save(&mut self, _irq_guard: &DisabledLocalIrqGuard) {
        self.fpu.save();
        self.cr2 = Cr2::read_raw();
        unsafe {
            self.run_msrs = RUN_MSRS.map(|index| msr::rdmsr(index));
        }
    }

    pub(super) fn load(&self, _irq_guard: &DisabledLocalIrqGuard) {
        write_cr2_raw(self.cr2);
        self.fpu.load();
        unsafe {
            for (index, value) in RUN_MSRS.into_iter().zip(self.run_msrs) {
                msr::wrmsr(index, value);
            }
        }
    }
}

const RUN_MSRS: [u32; 5] = [
    IA32_STAR,
    IA32_LSTAR,
    IA32_CSTAR,
    IA32_FMASK,
    IA32_KERNEL_GSBASE,
];
