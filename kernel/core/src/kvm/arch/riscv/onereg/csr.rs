// SPDX-License-Identifier: MPL-2.0

use crate::kvm::arch::vcpu::VcpuArch;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK, KVM_REG_RISCV_SUBTYPE_SHIFT, KVM_REG_RISCV_SUBTYPE_MASK, KVM_REG_RISCV, KVM_REG_SIZE_U64};
use super::KVM_REG_RISCV_CSR;
use ostd::arch::vm::{VSIP_TO_HVIP_SHIFT, VSIP_VALID_MASK, IRQ_LOCAL_MASK};
use crate::prelude::*;
use ostd::arch::vm::KvmVcpuCsr;

const KVM_REG_RISCV_CSR_GENERAL: usize = 0x0 << KVM_REG_RISCV_SUBTYPE_SHIFT;
const KVM_REG_RISCV_CSR_AIA: usize = 0x1 << KVM_REG_RISCV_SUBTYPE_SHIFT;
const KVM_REG_RISCV_CSR_SMSTATEEN: usize = 0x2 << KVM_REG_RISCV_SUBTYPE_SHIFT;

const KVM_REG_CSR_SIP: usize = 7;

impl VcpuArch {
    pub(super) fn get_csr(&self, reg_id: usize) -> Result<usize> {
        let mut reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CSR);
        let reg_subtype = KVM_REG_RISCV_SUBTYPE_MASK & reg_num;
        reg_num &= !(KVM_REG_RISCV_SUBTYPE_MASK);
        // Subtypes not yet impled: KVM_REG_RISCV_CSR_AIA, KVM_REG_RISCV_CSR_SMSTATEEN
        match reg_subtype {
            KVM_REG_RISCV_CSR_GENERAL => self.get_general_csr(reg_num),
            _ => Err(Error::new(Errno::ENOENT)),
        }
    }

    fn get_general_csr(&self, reg_num: usize) -> Result<usize> {
        match reg_num {
            KVM_REG_CSR_SIP => Ok(((self.inner.lock().guest_csr.hvip >> VSIP_TO_HVIP_SHIFT) & VSIP_VALID_MASK) & IRQ_LOCAL_MASK),
            _ => Ok(self.csr_reg(reg_num)?)
        }
    }

    pub(super) fn set_csr(&self, reg_id: usize, reg_val: usize) -> Result<()> {
        let mut reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CSR);
        let reg_subtype = KVM_REG_RISCV_SUBTYPE_MASK & reg_num;
        reg_num &= !(KVM_REG_RISCV_SUBTYPE_MASK);
        // Subtypes not yet impled: KVM_REG_RISCV_CSR_AIA, KVM_REG_RISCV_CSR_SMSTATEEN
        match reg_subtype {
            KVM_REG_RISCV_CSR_GENERAL => self.set_general_csr(reg_num, reg_val)?,
            _ => return Err(Error::new(Errno::ENOENT)),
        }
        Ok(())
    }

    fn set_general_csr(&self, reg_num: usize, reg_val: usize) -> Result<()> {
        match reg_num {
            KVM_REG_CSR_SIP => {
                let reg_val = (reg_val & VSIP_VALID_MASK) << VSIP_TO_HVIP_SHIFT;
                self.set_csr_reg(reg_num, reg_val)?;
            }
            _ => self.set_csr_reg(reg_num, reg_val)?,
        }
        Ok(())
    }

    pub(super) fn copy_csr_reg_vec(&self, mut vec: Option<&mut Vec<usize>>) -> Result<usize> {
        let mut n = 0;
        for i in 0..size_of::<KvmVcpuCsr>()/ size_of::<usize>() {
            let reg = KVM_REG_RISCV | KVM_REG_SIZE_U64 | KVM_REG_RISCV_CSR | i;
            if let Some(vec) = vec.as_deref_mut() {
                vec.push(reg);
            }
            n+=1;
        }
        Ok(n)
    }

    pub(super) fn num_csr_regs(&self) -> Result<usize> {
        self.copy_csr_reg_vec(None)
    }
}


