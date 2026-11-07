// SPDX-License-Identifier: MPL-2.0

use crate::kvm::arch::vcpu::VcpuArch;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK};
use super::{KVM_REG_RISCV_CONFIG, KVM_REG_RISCV, KVM_REG_SIZE_U64};
use crate::kvm::arch::isa::KVM_RISCV_BASE_ISA_MASK;
use ostd::arch::cmo_size;
use ostd::arch::cpu::extension::{has_extensions, IsaExtensions};
use ostd::arch::vm::{HGATP_MODE_48, HGATP_MODE_SHIFT};
use crate::prelude::*;

struct KvmConfig {
    isa: usize,
    zicbom_block_size: usize,
    mvendorid: usize,
    marchid: usize,
    mimpid: usize,
    zicboz_block_size: usize,
    satp_mode: usize,
    zicbop_block_size: usize,
}

impl KvmConfig {
    const ISA: usize = 0;
    const ZICBOM_BLOCK_SIZE: usize = Self::ISA + 1;
    const MVENDORID: usize = Self::ZICBOM_BLOCK_SIZE + 1;
    const MARCHID: usize = Self::MVENDORID + 1;
    const MIMPID: usize = Self::MARCHID + 1;
    const ZICBOZ_BLOCK_SIZE: usize = Self::MIMPID + 1;
    const SATP_MODE: usize = Self::ZICBOZ_BLOCK_SIZE + 1;
    const ZICBOP_BLOCK_SIZE: usize = Self::SATP_MODE + 1;
}

impl VcpuArch {
    pub(super) fn get_config(&self, reg_id: usize) -> Result<usize> {
        match reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CONFIG) {
            // Here, We only get low 64 bit of isa
            KvmConfig::ISA => Ok(KVM_RISCV_BASE_ISA_MASK & *self.isa.lock() as usize),
            KvmConfig::ZICBOM_BLOCK_SIZE => {
                if has_extensions(IsaExtensions::ZICBOM) 
                    {Ok(cmo_size())}
                else 
                    {Ok(0)}
            }
            KvmConfig::MVENDORID => Ok(self.inner.lock().mvendorid),
            KvmConfig::MARCHID => Ok(self.inner.lock().marchid),
            KvmConfig::MIMPID => Ok(self.inner.lock().mimpid),
            KvmConfig::ZICBOZ_BLOCK_SIZE => Ok(0),
            KvmConfig::SATP_MODE => Ok(HGATP_MODE_48 >> HGATP_MODE_SHIFT),
            KvmConfig::ZICBOP_BLOCK_SIZE => Ok(0),
            _ => Err(Error::new(Errno::ENOENT)),
        }
    }

    pub(super) fn copy_config_reg_vec(&self, mut vec: Option<&mut Vec<usize>>) -> Result<usize> {
        let mut n = 0;
        for i in 0..(size_of::<KvmConfig>()/ size_of::<usize>()) {
            let reg = KVM_REG_RISCV | KVM_REG_SIZE_U64 | KVM_REG_RISCV_CONFIG | i;
            if let Some(vec) = vec.as_deref_mut() {
                vec.push(reg);
            }
            n+=1;
        }
        Ok(n)
    }

    pub(super) fn num_config_regs(&self) -> Result<usize> {
        self.copy_config_reg_vec(None)
    }
}
