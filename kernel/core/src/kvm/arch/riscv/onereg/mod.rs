// SPDX-License-Identifier: MPL-2.0

use ::core::mem::offset_of;
use super::vcpu::VcpuArch;
use crate::prelude::*;
use ostd::arch::vm::VcpuContext;

mod config;
mod core;
mod csr;
mod fp_d;
mod fp_f;
mod timer;

// How do I add a new onereg type?
// 
// types not yet added: ISA_EXT, SBI_EXT, VECTOR, SBI_STATE
// funcs need to be impled: get(), set(), nums(), copy_to_vec()
//
// You can refer to the existing implementation to complete the rest.
//
// The expected types when fully implemented is 10, now is 6

const KVM_REG_ARCH_MASK: usize = 0xff00000000000000;

const KVM_REG_RISCV:usize = 0x8000000000000000;
/// The size mask of regs
pub const KVM_REG_SIZE_MASK: usize = 0x00f0000000000000;
/// The size of reg is u32
pub const KVM_REG_SIZE_U32:usize = 0x0020000000000000;
/// The size of reg is u64
pub const KVM_REG_SIZE_U64:usize = 0x0030000000000000;

const KVM_REG_RISCV_TYPE_MASK: usize = 0x00000000FF000000;
const KVM_REG_RISCV_TYPE_SHIFT: usize = 24;
const KVM_REG_RISCV_SUBTYPE_MASK: usize = 0x0000000000FF0000;
const KVM_REG_RISCV_SUBTYPE_SHIFT: usize = 16;

const KVM_REG_RISCV_CONFIG: usize = 0x01 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_CORE: usize = 0x02 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_CSR: usize = 0x03 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_TIMER: usize = 0x04 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_FP_F: usize = 0x05 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_FP_D: usize = 0x06 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_ISA_EXT: usize = 0x07 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_SBI_EXT: usize = 0x08 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_VECTOR: usize = 0x09 << KVM_REG_RISCV_TYPE_SHIFT;
const KVM_REG_RISCV_SBI_STATE: usize = 0x0A << KVM_REG_RISCV_TYPE_SHIFT;

// Logical register index layout for the RISC-V vCPU context.
//
// These indices are the flat, stable numbering used by the KVM ABI.
// They do NOT directly index any sub-array; instead, each index maps to
// a `(byte_offset, width)` pair against `VcpuContext::as_bytes()`.
//
// The layout mirrors `VcpuContext` field-for-field:
//
//   [CORE_REGS_BASE, CORE_REGS_END)   -> x0..x31 + sepc + sstatus + hstatus
//   [FP_F_REGS_BASE, FP_F_REGS_END)   -> f0..f31 + fcsr (u32 each)
//   [FP_D_REGS_BASE, FP_D_REGS_END)   -> f0..f31 (u64 each)
//   [FP_D_FCSR_INDEX]                 -> D-extension fcsr (u32)
//   [FP_Q_REGS_BASE, FP_Q_REGS_END)   -> f0..f63 (u64 each)
//   [FP_Q_FCSR_INDEX]                 -> Q-extension fcsr (u32)
const CORE_REGS_LEN: usize = 35;
const CORE_REGS_BASE: usize = 0;
const CORE_REGS_END: usize = CORE_REGS_BASE + CORE_REGS_LEN;

// 32 x u32 `f` + 1 x u32 `fcsr`.
const FP_F_REGS_LEN: usize = 33;
const FP_F_REGS_BASE: usize = CORE_REGS_END;
const FP_F_REGS_END: usize = FP_F_REGS_BASE + FP_F_REGS_LEN;

// 32 x u64 `f`.
const FP_D_REGS_LEN: usize = 32;
const FP_D_REGS_BASE: usize = FP_F_REGS_END;
const FP_D_REGS_END: usize = FP_D_REGS_BASE + FP_D_REGS_LEN;

const FP_D_FCSR_INDEX: usize = FP_D_REGS_END;

// 64 x u64 `f`.
const FP_Q_REGS_LEN: usize = 64;
const FP_Q_REGS_BASE: usize = FP_D_FCSR_INDEX + 1;
const FP_Q_REGS_END: usize = FP_Q_REGS_BASE + FP_Q_REGS_LEN;

const FP_Q_FCSR_INDEX: usize = FP_Q_REGS_END;

// Byte offsets of each logical register group inside `VcpuContext`.
// They are derived from the struct field layout so the compiler checks them.
const CORE_REGS_OFFSET: usize = 0;
const FP_F_REGS_OFFSET: usize = offset_of!(VcpuContext, fp_fctx);
const FP_D_REGS_OFFSET: usize = offset_of!(VcpuContext, fp_dctx);
const FP_D_FCSR_OFFSET: usize = FP_D_REGS_OFFSET + FP_D_REGS_LEN * size_of::<u64>();
const FP_Q_REGS_OFFSET: usize = offset_of!(VcpuContext, fp_qctx);
const FP_Q_FCSR_OFFSET: usize = FP_Q_REGS_OFFSET + FP_Q_REGS_LEN * size_of::<u64>();

const _: () = {
    // `VcpuContext` is `#[repr(C)]`, so the field offsets must agree
    // with the byte layout assumed by `reg`/`set_reg`.
    assert!(FP_F_REGS_OFFSET == CORE_REGS_LEN * size_of::<usize>());

    // `DFpuContext` holds 32 u64 registers; its `fcsr` must fit before the
    // Q-extension starts.
    assert!(FP_D_REGS_OFFSET >= FP_F_REGS_OFFSET + FP_F_REGS_LEN * size_of::<u32>());
    assert!(FP_D_FCSR_OFFSET + size_of::<u32>() <= FP_Q_REGS_OFFSET);

    // `QFpuContext` holds 64 u64 registers; its `fcsr` must fit inside the
    // struct.
    assert!(FP_Q_FCSR_OFFSET + size_of::<u32>() <= size_of::<VcpuContext>());
};

impl VcpuArch {
    /// Translate a logical register index into `(byte offset, register width)`.
    fn index_to_offset(index: usize) -> Result<(usize, usize)> {
        match index {
            CORE_REGS_BASE..CORE_REGS_END => Ok((
                CORE_REGS_OFFSET + (index - CORE_REGS_BASE) * size_of::<usize>(),
                size_of::<usize>(),
            )),
            FP_F_REGS_BASE..FP_F_REGS_END => Ok((
                FP_F_REGS_OFFSET + (index - FP_F_REGS_BASE) * size_of::<u32>(),
                size_of::<u32>(),
            )),
            FP_D_REGS_BASE..FP_D_REGS_END => Ok((
                FP_D_REGS_OFFSET + (index - FP_D_REGS_BASE) * size_of::<u64>(),
                size_of::<u64>(),
            )),
            FP_D_FCSR_INDEX => Ok((FP_D_FCSR_OFFSET, size_of::<u32>())),
            FP_Q_REGS_BASE..FP_Q_REGS_END => Ok((
                FP_Q_REGS_OFFSET + (index - FP_Q_REGS_BASE) * size_of::<u64>(),
                size_of::<u64>(),
            )),
            FP_Q_FCSR_INDEX => Ok((FP_Q_FCSR_OFFSET, size_of::<u32>())),
            _ => Err(Error::new(Errno::ENOENT)),
        }
    }

    /// Get the value of the corresponding register using a logical index.
    ///
    /// The index layout is described by the `*_BASE` / `*_LEN` constants above;
    /// each index maps to a byte offset inside `VcpuContext`.
    pub fn general_reg(&self, index: usize) -> Result<usize> {
        let (offset, width) = Self::index_to_offset(index)?;

        let guard = self.inner.lock();
        let bytes = guard.guest_context.as_bytes();

        match width {
        4 => {
            let arr: [u8; 4] = bytes
                .get(offset..offset + 4)
                .ok_or_else(|| Error::new(Errno::ENOENT))?
                .try_into()
                .map_err(|_| Error::new(Errno::ENOENT))?;
            Ok(u32::from_ne_bytes(arr) as usize)
        }
        8 => {
            let arr: [u8; 8] = bytes
                .get(offset..offset + 8)
                .ok_or_else(|| Error::new(Errno::ENOENT))?
                .try_into()
                .map_err(|_| Error::new(Errno::ENOENT))?;
            Ok(u64::from_ne_bytes(arr) as usize)
        }
        _ => Err(Error::new(Errno::ENOENT)),
        }
    }

    /// Set reg by logical index.
    ///
    /// See [`Self::reg`] for the index layout.
    pub fn set_general_reg(&self, index: usize, value: usize) -> Result<()> {
        let (offset, width) = Self::index_to_offset(index)?;
        let mut guard = self.inner.lock();
        let bytes = guard.guest_context.as_mut_bytes();
        match width {
            4 => bytes[offset..offset + 4].copy_from_slice(&(value as u32).to_ne_bytes()),
            8 => bytes[offset..offset + 8].copy_from_slice(&(value as u64).to_ne_bytes()),
            _ => return Err(Error::new(Errno::ENOENT)),
        }
        Ok(())
    }
}

impl VcpuArch {
    // SAFETY: Access to union members is safe.
    pub fn csr_reg(&self, index: usize) -> Result<usize> {
		let offset = index * 8;
		let guard = self.inner.lock();
        let bytes = guard.guest_csr.as_bytes();
        Ok(u64::from_ne_bytes(
            bytes[offset..offset + 8]
                .try_into()
                .unwrap(),
        ) as usize)
    }

    // SAFETY: Access to union members is safe.
    pub fn set_csr_reg(&self, index: usize, value: usize) -> Result<()> {
		let offset = index * 8;

        let mut guard = self.inner.lock();
        let bytes = guard.guest_csr.as_mut_bytes();

        bytes[offset..offset + 8].copy_from_slice(&(value as u64).to_ne_bytes());
        Ok(())
    }
}

impl VcpuArch {
    /// Get one reg by reg_id
    pub fn get_one_reg(&self, reg_id: usize) -> Result<usize> {
        match reg_id & KVM_REG_RISCV_TYPE_MASK {
            KVM_REG_RISCV_CONFIG => self.get_config(reg_id),
            KVM_REG_RISCV_CORE => self.get_core(reg_id),
            KVM_REG_RISCV_CSR => self.get_csr(reg_id),
            KVM_REG_RISCV_TIMER => self.get_timer(reg_id),
            KVM_REG_RISCV_FP_F => self.get_fp_f(reg_id),
            KVM_REG_RISCV_FP_D => self.get_fp_d(reg_id),
            _ => Err(Error::new(Errno::ENOENT))
        }
    }

    /// Set one reg by reg_id
    pub fn set_one_reg(&self, reg_id: usize, reg_val: usize) -> Result<()> {
        match reg_id & KVM_REG_RISCV_TYPE_MASK {
            KVM_REG_RISCV_CORE => self.set_core(reg_id, reg_val),
            KVM_REG_RISCV_CSR => self.set_csr(reg_id, reg_val),
            KVM_REG_RISCV_FP_F => self.set_fp_f(reg_id, reg_val),
            KVM_REG_RISCV_FP_D => self.set_fp_d(reg_id, reg_val),
            _ => Err(Error::new(Errno::ENOENT))
        }
    }

    /// Get numbers of regs which can be used in GetOneReg/SetOneReg
    pub fn num_regs(&self) -> Result<usize> {
        let mut res = 0;

        res+=self.num_config_regs()?;
        res+=self.num_core_regs()?;
        res+=self.num_csr_regs()?;
        res+=self.num_timer_regs()?;
        res+=self.num_fp_f_regs()?;
        res+=self.num_fp_d_regs()?;

        Ok(res)
    }

    /// Get reg_id of regs which can be used in GetOneReg/SetOneReg
    pub fn copy_reg_vec(&self, mut vec: Option<&mut Vec<usize>>) -> Result<()> {
        self.copy_config_reg_vec(vec.as_deref_mut())?;
        self.copy_core_reg_vec(vec.as_deref_mut())?;
        self.copy_csr_reg_vec(vec.as_deref_mut())?;
        self.copy_timer_reg_vec(vec.as_deref_mut())?;
        self.copy_fp_f_reg_vec(vec.as_deref_mut())?;
        self.copy_fp_d_reg_vec(vec.as_deref_mut())?;

        Ok(())
    }
}
