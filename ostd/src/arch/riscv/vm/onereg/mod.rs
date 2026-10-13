use super::vcpu::VcpuArchInner;
use crate::prelude::*;
use super::isa::KVM_RISCV_BASE_ISA_MASK;

mod config;
mod core;
mod csr;
mod fp_d;
mod fp_f;
mod isa_ext;
mod sbi_ext;
mod sbi_state;
mod timer;
mod vector;

pub(super) use csr::KvmVcpuCsr;

const KVM_REG_ARCH_MASK: usize = 0xff00000000000000;
const KVM_REG_SIZE_MASK: usize = 0x00f0000000000000;

const KVM_REG_RISCV_TYPE_MASK: usize = 0x00000000FF000000;
const KVM_REG_RISCV_TYPE_SHIFT: usize = 24;
const KVM_REG_RISCV_SUBTYPE_MASK: usize = 0x0000000000FF0000;
pub(super) const KVM_REG_RISCV_SUBTYPE_SHIFT: usize = 16;

pub(super)const KVM_REG_RISCV:usize = 0x8000000000000000;
const KVM_REG_SIZE_U32:usize = 0x0020000000000000;
pub(super)const KVM_REG_SIZE_U64:usize = 0x0030000000000000;

const KVM_REG_RISCV_CONFIG: usize = 0x01 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_CORE: usize = 0x02 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_CSR: usize = 0x03 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_TIMER: usize = 0x04 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_FP_F: usize = 0x05 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_FP_D: usize = 0x06 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_ISA_EXT: usize = 0x07 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_SBI_EXT: usize = 0x08 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_VECTOR: usize = 0x09 << KVM_REG_RISCV_TYPE_SHIFT;
pub(super)const KVM_REG_RISCV_SBI_STATE: usize = 0x0A << KVM_REG_RISCV_TYPE_SHIFT;

impl VcpuArchInner {
    /// 1
    pub fn get_one_reg(&self, reg_id: usize) -> usize {
        match reg_id & KVM_REG_RISCV_TYPE_MASK {
            KVM_REG_RISCV_CONFIG => self.get_config(reg_id),
            KVM_REG_RISCV_CORE => self.get_core(reg_id),
            KVM_REG_RISCV_CSR => self.get_csr(reg_id),
            KVM_REG_RISCV_TIMER => self.get_timer(reg_id),
            KVM_REG_RISCV_FP_F => self.get_fp_f(reg_id),
            KVM_REG_RISCV_FP_D => self.get_fp_d(reg_id),
            KVM_REG_RISCV_ISA_EXT => self.get_isa_ext(reg_id),
            KVM_REG_RISCV_SBI_EXT => self.get_sbi_ext(reg_id),
            KVM_REG_RISCV_VECTOR => self.get_vector(reg_id),
            KVM_REG_RISCV_SBI_STATE => self.get_sbi_state(reg_id),
            _ => panic!("unknown onereg id!")
        }
    }

    /// 1
    pub fn set_one_reg(&mut self, reg_id: usize, reg_val: usize) {
        match reg_id & KVM_REG_RISCV_TYPE_MASK {
            KVM_REG_RISCV_CONFIG => self.set_config(reg_id, reg_val),
            KVM_REG_RISCV_CORE => self.set_core(reg_id, reg_val),
            KVM_REG_RISCV_CSR => self.set_csr(reg_id, reg_val),
            KVM_REG_RISCV_TIMER => self.set_timer(reg_id, reg_val),
            KVM_REG_RISCV_FP_F => self.set_fp_f(reg_id, reg_val),
            KVM_REG_RISCV_FP_D => self.set_fp_d(reg_id, reg_val),
            KVM_REG_RISCV_ISA_EXT => self.set_isa_ext(reg_id, reg_val),
            KVM_REG_RISCV_SBI_EXT => self.set_sbi_ext(reg_id, reg_val),
            KVM_REG_RISCV_VECTOR => self.set_vector(reg_id, reg_val),
            KVM_REG_RISCV_SBI_STATE => self.set_sbi_state(reg_id, reg_val),
            _ => panic!("unknown onereg id!")
        }
    }

    /// 2
    pub fn num_regs(&self) -> usize {
        let mut res = 0;

        res+=self.num_config_regs();
        res+=self.num_core_regs();
        res+=self.num_csr_regs();
        res+=self.num_timer_regs();
        res+=self.num_fp_f_regs();
        res+=self.num_fp_d_regs();
        res+=self.num_vector_regs();
        res+=self.num_isa_ext_regs();
        res+=self.num_sbi_ext_regs();
        res+=self.num_sbi_state_regs();

        res
    }

    /// 3
    pub fn copy_reg_vec(&self, mut vec: Option<&mut Vec<usize>>) {
        self.copy_config_reg_vec(vec.as_deref_mut());
        self.copy_core_reg_vec(vec.as_deref_mut());
        self.copy_csr_reg_vec(vec.as_deref_mut());
        self.copy_timer_reg_vec(vec.as_deref_mut());
        self.copy_fp_f_reg_vec(vec.as_deref_mut());
        self.copy_fp_d_reg_vec(vec.as_deref_mut());
        self.copy_vector_reg_vec(vec.as_deref_mut());
        self.copy_isa_ext_reg_vec(vec.as_deref_mut());
        self.copy_sbi_ext_reg_vec(vec.as_deref_mut());
        self.copy_sbi_state_reg_vec(vec.as_deref_mut());
    }
}
