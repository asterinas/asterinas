// SPDX-License-Identifier: MPL-2.0

use crate::kvm::arch::vcpu::VcpuArch;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK, KVM_REG_RISCV, KVM_REG_SIZE_U64};
use super::KVM_REG_RISCV_FP_D;
use crate::prelude::*;

impl VcpuArch {
    pub(super) fn get_fp_d(&self, reg_id: usize) -> Result<usize> {
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_FP_D);
        match reg_num {
            32 => Ok(self.inner.lock().guest_context.fp_dctx.fcsr() as usize),
            0..32 => Ok(self.inner.lock().guest_context.fp_dctx.f(reg_num) as usize),
            _ => Err(Error::new(Errno::ENOENT)),
        }
    }

    pub(super) fn set_fp_d(&self, reg_id: usize, reg_val: usize) -> Result<()> {
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_FP_D);
        match reg_num {
            32 => self.inner.lock().guest_context.fp_dctx.set_fcsr(reg_val as u32),
            0..32 =>self.inner.lock().guest_context.fp_dctx.set_f(reg_num, reg_val as u64),
            _ => return Err(Error::new(Errno::ENOENT)),
        }
        Ok(())
    }

    pub(super) fn copy_fp_d_reg_vec(&self, mut vec: Option<&mut Vec<usize>>) -> Result<usize> {
        let mut n = 0;
        for i in 0..32 {
            let reg = KVM_REG_RISCV | KVM_REG_SIZE_U64 | KVM_REG_RISCV_FP_D | i;
            if let Some(vec) = vec.as_deref_mut() {
                vec.push(reg);
            }
            n+=1;
        }

        let reg = KVM_REG_RISCV | KVM_REG_SIZE_U64 | KVM_REG_RISCV_FP_D | 32;
        if let Some(vec) = vec.as_deref_mut() {
            vec.push(reg);
        }
        n+=1;
        Ok(n)
    }

    pub(super) fn num_fp_d_regs(&self) -> Result<usize> {
        self.copy_fp_d_reg_vec(None)
    }
}
