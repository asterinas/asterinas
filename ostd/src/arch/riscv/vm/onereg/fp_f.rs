use crate::arch::vm::vcpu::VcpuArchInner;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK, KVM_REG_RISCV, KVM_REG_SIZE_U32};
use super::KVM_REG_RISCV_FP_F;
use crate::arch::cpu::context::FFpuContext;
use crate::prelude::*;

impl VcpuArchInner {
    pub(super) fn get_fp_f(&self, reg_id: usize) -> usize {
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_FP_F);
        match reg_num {
            32 => {
                self.guest_context.get_fp_fctx_fcsr()
            }
            0..32 => {
                self.guest_context.get_fp_fctx_f(reg_num)
            }
            _ => {
                panic!("unknown fp_f reg id!");
            }
        }
    }

    pub(super) fn set_fp_f(&mut self, reg_id: usize, reg_val: usize) {
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_FP_F);
        match reg_num {
            32 => {
                self.guest_context.set_fp_fctx_fcsr(reg_val);
            }
            0..32 => {
                self.guest_context.set_fp_fctx_f(reg_num, reg_val);
            }
            _ => {
                panic!("unknown fp_f reg id: {}", reg_num);
            }
        }
    }

    pub(super) fn copy_fp_f_reg_vec(&self, mut vec: Option<&mut Vec<usize>>){
        let n = self.num_fp_f_regs();
        for i in 0..n {
            let reg = KVM_REG_RISCV as usize 
                            | KVM_REG_SIZE_U32 as usize 
                            | KVM_REG_RISCV_FP_F as usize
                            | i;
            if let Some(vec) = vec.as_deref_mut() {
                vec.push(reg);
            }
        }
    }

    pub(super) fn num_fp_f_regs(&self) -> usize {
        size_of::<FFpuContext>()/ size_of::<u32>()
    }
}