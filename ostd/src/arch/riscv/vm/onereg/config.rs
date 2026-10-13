use crate::arch::vm::vcpu::VcpuArchInner;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK};
use super::{KVM_REG_RISCV_CONFIG, KVM_RISCV_BASE_ISA_MASK, KVM_REG_RISCV, KVM_REG_SIZE_U64};
use core::mem::offset_of;
use crate::arch::mm::cmo_block_size;
use crate::arch::cpu::extension::{has_extensions, IsaExtensions};
use crate::arch::vm::types::{SATP_MODE_48, SATP_MODE_SHIFT};
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
    const ISA: usize = offset_of!(KvmConfig, isa);
    const ZICBOM_BLOCK_SIZE: usize = Self::ISA + 1;
    const MVENDORID: usize = Self::ZICBOM_BLOCK_SIZE + 1;
    const MARCHID: usize = Self::MVENDORID + 1;
    const MIMPID: usize = Self::MARCHID + 1;
    const ZICBOZ_BLOCK_SIZE: usize = Self::MIMPID + 1;
    const SATP_MODE: usize = Self::ZICBOZ_BLOCK_SIZE + 1;
    const ZICBOP_BLOCK_SIZE: usize = Self::SATP_MODE + 1;
}

impl VcpuArchInner {
    pub(super) fn get_config(&self, reg_id: usize) -> usize {
        match reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CONFIG) {
            KvmConfig::ISA => {
                KVM_RISCV_BASE_ISA_MASK & self.isa[0] as usize
            }
            KvmConfig::ZICBOM_BLOCK_SIZE => {
                if has_extensions(IsaExtensions::ZICBOM) 
                    {cmo_block_size()}
                else 
                    {0}
            }
            KvmConfig::MVENDORID => {
                self.mvendorid
            }
            KvmConfig::MARCHID => {
                self.marchid
            }
            KvmConfig::MIMPID => {
                self.mimpid
            }
            KvmConfig::ZICBOZ_BLOCK_SIZE => {
                0
            }
            KvmConfig::SATP_MODE => {
                (SATP_MODE_48 >> SATP_MODE_SHIFT) as usize
            }
            KvmConfig::ZICBOP_BLOCK_SIZE => {
                0
            }
            _ => {
                panic!("unknown config reg id!");
            }
        }
    }

    pub(super) fn set_config(&self, _reg_id: usize, _reg_val: usize) {
        todo!("Onereg: set_config is not impled!")
    }

    pub(super) fn copy_config_reg_vec(&self, mut vec: Option<&mut Vec<usize>>) -> usize {
        let mut n = 0;
        for i in 0..(size_of::<KvmConfig>()/ size_of::<usize>()) {
            let reg = KVM_REG_RISCV as usize 
                            | KVM_REG_SIZE_U64 as usize 
                            | KVM_REG_RISCV_CONFIG as usize
                            | i;
            if let Some(vec) = vec.as_deref_mut() {
                vec.push(reg);
            }
            n+=1;
        }
        n
    }

    pub(super) fn num_config_regs(&self) -> usize {
        self.copy_config_reg_vec(None)
    }
}
