use crate::arch::vm::vcpu::VcpuArchInner;
use super::{KVM_REG_ARCH_MASK, KVM_REG_SIZE_MASK, KVM_REG_RISCV_SUBTYPE_SHIFT, KVM_REG_RISCV_SUBTYPE_MASK, KVM_REG_RISCV, KVM_REG_SIZE_U64};
use super::KVM_REG_RISCV_CSR;
use crate::arch::vm::types::{VSIP_TO_HVIP_SHIFT, VSIP_VALID_MASK, IRQ_LOCAL_MASK};
use crate::prelude::*;

const KVM_REG_RISCV_CSR_GENERAL: usize = 	0x0 << KVM_REG_RISCV_SUBTYPE_SHIFT;
const KVM_REG_RISCV_CSR_AIA: usize = 0x1 << KVM_REG_RISCV_SUBTYPE_SHIFT;
const KVM_REG_RISCV_CSR_SMSTATEEN: usize = 0x2 << KVM_REG_RISCV_SUBTYPE_SHIFT;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in crate::arch::vm) struct KvmVcpuCsr {
	pub vsstatus: usize,
	pub vsie: usize,
	pub vstvec: usize,
	pub vsscratch: usize,
	pub vsepc: usize,
	pub vscause: usize,
	pub vstval: usize,
	pub hvip: usize,
	pub vsatp: usize,
	pub scounteren: usize,
	pub senvcfg: usize,
}

impl KvmVcpuCsr {
    const SIP: usize = 7;
}

impl KvmVcpuCsr {
    /// 通过索引获取 CSR 值
    fn get_csr(&self, index: u64) -> Option<usize> {
        match index {
            0 => Some(self.vsstatus),
            1 => Some(self.vsie),
            2 => Some(self.vstvec),
            3 => Some(self.vsscratch),
            4 => Some(self.vsepc),
            5 => Some(self.vscause),
            6 => Some(self.vstval),
            7 => Some(self.hvip),
            8 => Some(self.vsatp),
            9 => Some(self.scounteren),
            10 => Some(self.senvcfg),
            _ => None,
        }
    }

    /// 通过索引设置 CSR 值
    fn set_csr(&mut self, index: usize, value: usize) -> bool {
        match index {
            0 => { self.vsstatus = value; true }
            1 => { self.vsie = value; true }
            2 => { self.vstvec = value; true }
            3 => { self.vsscratch = value; true }
            4 => { self.vsepc = value; true }
            5 => { self.vscause = value; true }
            6 => { self.vstval = value; true }
            7 => { self.hvip = value; true }
            8 => { self.vsatp = value; true }
            9 => { self.scounteren = value; true }
            10 => { self.senvcfg = value; true }
            _ => false,
        }
    }
}

impl VcpuArchInner {
    pub(super) fn get_csr(&self, reg_id: usize) -> usize{
        let mut reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CSR);
        let reg_subtype = KVM_REG_RISCV_SUBTYPE_MASK & reg_num;
        reg_num &= !(KVM_REG_RISCV_SUBTYPE_MASK);
        match reg_subtype {
            KVM_REG_RISCV_CSR_GENERAL => {
                self.get_general_csr(reg_num)
            }
            KVM_REG_RISCV_CSR_AIA => {
                panic!("SSAIA isa support is disabled!");
            }
            KVM_REG_RISCV_CSR_SMSTATEEN => {
                panic!("SMSTATEEN isa support is disabled!");
            }
            _ => {
                panic!("unknown csr reg id!");
            }
        }
    }

    fn get_general_csr(&self, reg_num: usize) -> usize {
        match reg_num {
            KvmVcpuCsr::SIP => {
                ((self.guest_csr.hvip >> VSIP_TO_HVIP_SHIFT) & VSIP_VALID_MASK as usize) & IRQ_LOCAL_MASK as usize
            }
            _ => {
                self.guest_csr.get_csr(reg_num as u64).unwrap()
            }
        }
    }

    pub(super) fn set_csr(&mut self, reg_id: usize, reg_val: usize){
        let mut reg_num = reg_id & !(KVM_REG_ARCH_MASK | KVM_REG_SIZE_MASK | KVM_REG_RISCV_CSR);
        let reg_subtype = KVM_REG_RISCV_SUBTYPE_MASK & reg_num;
        reg_num &= !(KVM_REG_RISCV_SUBTYPE_MASK);
        match reg_subtype {
            KVM_REG_RISCV_CSR_GENERAL => {
                self.set_general_csr(reg_num, reg_val)
            }
            KVM_REG_RISCV_CSR_AIA => {
                panic!("SSAIA isa support is disabled!");
            }
            KVM_REG_RISCV_CSR_SMSTATEEN => {
                panic!("SMSTATEEN isa support is disabled!");
            }
            _ => {
                panic!("unknown csr reg id!");
            }
        }
    }

    fn set_general_csr(&mut self, reg_num: usize, reg_val: usize) {
        match reg_num {
            KvmVcpuCsr::SIP => {
                let reg_val = (reg_val & VSIP_VALID_MASK as usize) << VSIP_TO_HVIP_SHIFT;
                self.guest_csr.set_csr(reg_num as usize, reg_val);
            }
            _ => {
                self.guest_csr.set_csr(reg_num as usize, reg_val);
            }
        }
    }

    pub(super) fn copy_csr_reg_vec(&self, mut vec: Option<&mut Vec<usize>>){
        let n = self.num_csr_regs();
        for i in 0..n {
            let reg = KVM_REG_RISCV as usize 
                            | KVM_REG_SIZE_U64 as usize 
                            | KVM_REG_RISCV_CSR as usize
                            | i;
            if let Some(vec) = vec.as_deref_mut() {
                vec.push(reg);
            }
        }
    }

    pub(super) fn num_csr_regs(&self) -> usize {
        // TODO: add vcpu SSAIA, SMSTATEEN isa ext
        size_of::<KvmVcpuCsr>()/ size_of::<usize>()
    }
}


