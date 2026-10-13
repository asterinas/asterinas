use crate::arch::vm::vcpu::VcpuArchInner;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK, KVM_REG_RISCV, KVM_REG_SIZE_U64};
use super::KVM_REG_RISCV_CORE;
use crate::arch::vm::csr::SR_SPP;
use crate::prelude::*;

struct UserRegs {
    // different to GeneralRegs, first is 'pc', not 'zero'
    pub pc: usize,
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
}

struct KvmCore {
	regs: UserRegs,
	mode: u64,
}

impl KvmCore {
    const PC: usize = 0;
    const T6: usize = Self::PC + 31;
    const MODE: usize = Self::PC + 32;
}


impl VcpuArchInner {
    pub(super) fn get_core(&self, reg_id: usize)->usize{
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CORE);
        match reg_num {
            KvmCore::PC => {
                self.guest_context.get_sepc()
            }
            KvmCore::MODE => {
                (self.guest_context.get_sstatus() & SR_SPP as usize == 1) as usize
            }
            KvmCore::PC..=KvmCore::T6 => {
                self.guest_context.get_reg(reg_num as u64).unwrap()
            }
            _ => {
                panic!("unknown core reg id!");
            }
        }
    }

    pub(super) fn set_core(&mut self, reg_id: usize, reg_val: usize){
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CORE);
        match reg_num {
            KvmCore::PC => {
                self.guest_context.set_sepc(reg_val);
            }
            KvmCore::MODE => {
                if reg_val == 1 {
                    let sstatus = self.guest_context.get_sstatus();
                    self.guest_context.set_sstatus(sstatus | SR_SPP);
                } else {
                    let sstatus = self.guest_context.get_sstatus();
                    self.guest_context.set_sstatus(sstatus & SR_SPP);
                }
            }
            KvmCore::PC..=KvmCore::T6 => {
                self.guest_context.set_reg(reg_num as usize, reg_val);
            }
            _ => {
                panic!("unknown core reg id: {:x}, {:x}", reg_id, reg_num);
            }
        }
    }

    pub(super) fn copy_core_reg_vec(&self, mut vec: Option<&mut Vec<usize>>){
        let n = self.num_core_regs();
        for i in 0..n {
            let reg = KVM_REG_RISCV as usize 
                            | KVM_REG_SIZE_U64 as usize 
                            | KVM_REG_RISCV_CORE as usize
                            | i;
            if let Some(vec) = vec.as_deref_mut() {
                vec.push(reg);
            }
        }
    }

    pub(super) fn num_core_regs(&self) -> usize {
        size_of::<KvmCore>()/ size_of::<usize>()
    }
}
