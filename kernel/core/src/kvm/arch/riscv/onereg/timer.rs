// SPDX-License-Identifier: MPL-2.0

use crate::kvm::arch::vcpu::VcpuArch;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK, KVM_REG_RISCV, KVM_REG_SIZE_U64};
use super::KVM_REG_RISCV_TIMER;
use ostd::arch::tsc_freq;
use crate::prelude::*;

struct KvmRiscvTimer {
	frequency: usize,
	time: usize,
	compare: usize,
	state: usize,
}

impl KvmRiscvTimer {
    const FREQUENCY: usize = 0;
    const TIME: usize = Self::FREQUENCY + 1;
    const COMPARE: usize = Self::TIME + 1;
    const STATE: usize = Self::COMPARE + 1;
}

impl VcpuArch {
    pub(super) fn get_timer(&self, reg_id: usize) -> Result<usize> {
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_TIMER);
        // regs not yet impled: TIME, COMPARE, STATE
        match reg_num {
            KvmRiscvTimer::FREQUENCY => Ok(tsc_freq() as usize),
            _ => Err(Error::new(Errno::ENOENT)),
        }
    }

    pub(super) fn copy_timer_reg_vec(&self, mut vec: Option<&mut Vec<usize>>) -> Result<usize> {
        let mut n = 0;
        for i in 0..size_of::<KvmRiscvTimer>()/ size_of::<usize>() {
            let reg = KVM_REG_RISCV | KVM_REG_SIZE_U64 | KVM_REG_RISCV_TIMER | i;
            if let Some(vec) = vec.as_deref_mut() {
                vec.push(reg);
            }
            n+=1;
        }
        Ok(n)
    }

    pub(super) fn num_timer_regs(&self) -> Result<usize> {
        self.copy_timer_reg_vec(None)
    }
}
