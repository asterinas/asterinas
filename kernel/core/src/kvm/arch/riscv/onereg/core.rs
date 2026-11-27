// SPDX-License-Identifier: MPL-2.0

use crate::kvm::arch::vcpu::VcpuArch;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK, KVM_REG_RISCV, KVM_REG_SIZE_U64};
use super::KVM_REG_RISCV_CORE;
use crate::prelude::*;
use riscv::register::sstatus::{Sstatus, SPP};

struct UserRegs {
    // different to GeneralRegs, first is 'pc', not 'zero'
    pc: usize,
    ra: usize,
    sp: usize,
    gp: usize,
    tp: usize,
    t0: usize,
    t1: usize,
    t2: usize,
    s0: usize,
    s1: usize,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
    a7: usize,
    s2: usize,
    s3: usize,
    s4: usize,
    s5: usize,
    s6: usize,
    s7: usize,
    s8: usize,
    s9: usize,
    s10: usize,
    s11: usize,
    t3: usize,
    t4: usize,
    t5: usize,
    t6: usize,
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

impl VcpuArch {
    pub(super) fn get_core(&self, reg_id: usize) -> Result<usize> {
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CORE);
        match reg_num {
            KvmCore::PC => Ok(self.inner.lock().guest_context.sepc),
            KvmCore::MODE => Ok((Sstatus::from_bits(self.inner.lock().guest_context.sstatus).spp() == SPP::Supervisor) as usize),
            KvmCore::PC..=KvmCore::T6 => Ok(self.general_reg(reg_num)?),
            _ => Err(Error::new(Errno::ENOENT)),
        }
    }

    pub(super) fn set_core(&self, reg_id: usize, reg_val: usize) -> Result<()> {
        let reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CORE);
        match reg_num {
            KvmCore::PC => self.inner.lock().guest_context.sepc+=reg_val,
            KvmCore::MODE => {
                let mut sstatus = Sstatus::from_bits(self.inner.lock().guest_context.sstatus);
                if reg_val == 1 {
                    sstatus.set_spp(SPP::Supervisor);
                } else {
                    sstatus.set_spp(SPP::User);
                }
                self.inner.lock().guest_context.sstatus = sstatus.bits();
            }
            KvmCore::PC..=KvmCore::T6 => self.set_general_reg(reg_num, reg_val)?,
            _ => return Err(Error::new(Errno::ENOENT))
        }
        Ok(())
    }

    pub(super) fn copy_core_reg_vec(&self, mut vec: Option<&mut Vec<usize>>) -> Result<usize> {
        let mut n = 0;
        for i in 0..size_of::<KvmCore>()/ size_of::<usize>() {
            let reg = KVM_REG_RISCV | KVM_REG_SIZE_U64 | KVM_REG_RISCV_CORE | i;
            if let Some(vec) = vec.as_deref_mut() {
                vec.push(reg);
            }
            n+=1;
        }
        Ok(n)
    }

    pub(super) fn num_core_regs(&self) -> Result<usize> {
        self.copy_core_reg_vec(None)
    }
}
