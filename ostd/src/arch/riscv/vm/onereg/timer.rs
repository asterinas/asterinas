use crate::arch::vm::vcpu::VcpuArchInner;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK};
use super::KVM_REG_RISCV_TIMER;
use crate::arch::timer::get_timebase_freq;
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

impl VcpuArchInner {
    pub(super) fn get_timer(&self, reg_id: usize) -> usize {
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_TIMER);
        match reg_num {
            KvmRiscvTimer::FREQUENCY => {
                get_timebase_freq() as usize
            }
            KvmRiscvTimer::TIME => {
                panic!("KvmRiscvTimer::TIME is disabled!");
            }
            KvmRiscvTimer::COMPARE => {
                panic!("KvmRiscvTimer::COMP is disabled!");
            }
            KvmRiscvTimer::STATE => {
                panic!("KvmRiscvTimer::STATE is disabled!");
            }
            _ => {
                panic!("unknown timer reg id!");
            }
        }
    }
    
    pub(super) fn set_timer(&self, _reg_id: usize, _reg_val: usize) {
        todo!("OneReg: set_timer is not impled!")
    }

    pub(super) fn copy_timer_reg_vec(&self, mut _vec: Option<&mut Vec<usize>>) -> usize {
        // todo!("OneReg: copy_timer_reg_vec is not impled!")
        0
    }

    pub(super) fn num_timer_regs(&self) -> usize {
        // todo!("OneReg: num_timer_regs is not impled")
        0
    }
}
