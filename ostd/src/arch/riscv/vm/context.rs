// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd_pod::IntoBytes;

use super::onereg::OneRegError;
use crate::{
    arch::cpu::context::{DFpuContext, FFpuContext, QFpuContext},
    const_assert,
    prelude::*,
};

/// The context of the vCPU, which contains several registers.
#[padding_struct]
#[repr(C)]
#[derive(Default, Clone, Copy, Pod)]
pub struct VcpuContext {
    zero: usize,
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
    sepc: usize,
    sstatus: usize,
    hstatus: usize,
    fp_fctx: FFpuContext,
    fp_dctx: DFpuContext,
    fp_qctx: QFpuContext,
}

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
const FP_F_REGS_OFFSET: usize = core::mem::offset_of!(VcpuContext, fp_fctx);
const FP_D_REGS_OFFSET: usize = core::mem::offset_of!(VcpuContext, fp_dctx);
const FP_D_FCSR_OFFSET: usize = FP_D_REGS_OFFSET + FP_D_REGS_LEN * size_of::<u64>();
const FP_Q_REGS_OFFSET: usize = core::mem::offset_of!(VcpuContext, fp_qctx);
const FP_Q_FCSR_OFFSET: usize = FP_Q_REGS_OFFSET + FP_Q_REGS_LEN * size_of::<u64>();

// `VcpuContext` is `#[repr(C)]`, so the field offsets must agree
// with the byte layout assumed by `reg`/`set_reg`.
const_assert!(FP_F_REGS_OFFSET == CORE_REGS_LEN * size_of::<usize>());

// `DFpuContext` holds 32 u64 registers; its `fcsr` must fit before the Q-extension starts.
const_assert!(FP_D_REGS_OFFSET >= FP_F_REGS_OFFSET + FP_F_REGS_LEN * size_of::<u32>());
const_assert!(FP_D_FCSR_OFFSET + size_of::<u32>() <= FP_Q_REGS_OFFSET);

// `QFpuContext` holds 64 u64 registers; its `fcsr` must fit inside the struct.
const_assert!(FP_Q_FCSR_OFFSET + size_of::<u32>() <= size_of::<VcpuContext>());

impl VcpuContext {
    /// Translate a logical register index into the byte range it occupies.
    fn index_to_offset(index: usize) -> Result<Range<usize>, OneRegError> {
        let (offset, width) = match index {
            CORE_REGS_BASE..CORE_REGS_END => (
                CORE_REGS_OFFSET + (index - CORE_REGS_BASE) * size_of::<usize>(),
                size_of::<usize>(),
            ),
            FP_F_REGS_BASE..FP_F_REGS_END => (
                FP_F_REGS_OFFSET + (index - FP_F_REGS_BASE) * size_of::<u32>(),
                size_of::<u32>(),
            ),
            FP_D_REGS_BASE..FP_D_REGS_END => (
                FP_D_REGS_OFFSET + (index - FP_D_REGS_BASE) * size_of::<u64>(),
                size_of::<u64>(),
            ),
            FP_D_FCSR_INDEX => (FP_D_FCSR_OFFSET, size_of::<u32>()),
            FP_Q_REGS_BASE..FP_Q_REGS_END => (
                FP_Q_REGS_OFFSET + (index - FP_Q_REGS_BASE) * size_of::<u64>(),
                size_of::<u64>(),
            ),
            FP_Q_FCSR_INDEX => (FP_Q_FCSR_OFFSET, size_of::<u32>()),
            _ => return Err(OneRegError::NotFoundRegister),
        };
        Ok(offset..offset + width)
    }

    /// Get the value of the corresponding register using a logical index.
    ///
    /// The index layout is described by the `*_BASE` / `*_LEN` constants above;
    /// each index maps to a byte range inside `VcpuContext`.
    pub fn reg(&self, index: usize) -> Result<usize, OneRegError> {
        let range = Self::index_to_offset(index)?;
        let width = range.len();
        let bytes = self.as_bytes();
        let slice = &bytes[range];
        match width {
            4 => Ok(u32::from_ne_bytes(
                slice
                    .try_into()
                    .map_err(|_| OneRegError::NotFoundRegister)?,
            ) as usize),
            8 => Ok(u64::from_ne_bytes(
                slice
                    .try_into()
                    .map_err(|_| OneRegError::NotFoundRegister)?,
            ) as usize),
            _ => Err(OneRegError::NotFoundRegister),
        }
    }

    /// Set reg by logical index.
    ///
    /// See [`Self::reg`] for the index layout.
    pub fn set_reg(&mut self, index: usize, value: usize) -> Result<(), OneRegError> {
        let range = Self::index_to_offset(index)?;
        let width = range.len();
        let bytes = self.as_mut_bytes();
        let slice = &mut bytes[range];
        match width {
            4 => slice.copy_from_slice(&(value as u32).to_ne_bytes()),
            8 => slice.copy_from_slice(&(value as u64).to_ne_bytes()),
            _ => return Err(OneRegError::NotFoundRegister),
        }
        Ok(())
    }

    pub(super) fn a0(&self) -> usize {
        self.a0
    }

    pub(super) fn set_a0(&mut self, a0: usize) {
        self.a0 = a0;
    }

    pub(super) fn a1(&self) -> usize {
        self.a1
    }

    pub(super) fn set_a1(&mut self, a1: usize) {
        self.a1 = a1;
    }

    pub(super) fn a6(&self) -> usize {
        self.a6
    }

    pub(super) fn set_a6(&mut self, a6: usize) {
        self.a6 = a6;
    }

    pub(super) fn a7(&self) -> usize {
        self.a7
    }

    pub(super) fn set_a7(&mut self, a7: usize) {
        self.a7 = a7;
    }

    pub(super) fn sepc(&self) -> usize {
        self.sepc
    }

    pub(super) fn set_sepc(&mut self, sepc: usize) {
        self.sepc = sepc;
    }

    /// Increases the SEPC register by several bytes.
    pub(super) fn step_sepc(&mut self, bytes: usize) {
        self.sepc += bytes;
    }

    pub(super) fn sstatus(&self) -> usize {
        self.sstatus
    }

    pub(super) fn set_sstatus(&mut self, sstatus: usize) {
        self.sstatus = sstatus;
    }

    pub(super) fn hstatus(&self) -> usize {
        self.hstatus
    }

    pub(super) fn set_hstatus(&mut self, hstatus: usize) {
        self.hstatus = hstatus;
    }

    pub(super) fn fp_fctx_f(&self, index: usize) -> usize {
        self.fp_fctx.f(index) as usize
    }

    pub(super) fn set_fp_fctx_f(&mut self, index: usize, f: usize) {
        self.fp_fctx.set_f(index, f as u32)
    }

    pub(super) fn fp_fctx_fcsr(&self) -> usize {
        self.fp_fctx.fcsr() as usize
    }

    pub(super) fn set_fp_fctx_fcsr(&mut self, fcsr: usize) {
        self.fp_fctx.set_fcsr(fcsr as u32)
    }

    pub(super) fn fp_dctx_f(&self, index: usize) -> usize {
        self.fp_dctx.f(index) as usize
    }

    pub(super) fn set_fp_dctx_f(&mut self, index: usize, f: usize) {
        self.fp_dctx.set_f(index, f as u64)
    }

    pub(super) fn fp_dctx_fcsr(&self) -> usize {
        self.fp_dctx.fcsr() as usize
    }

    pub(super) fn set_fp_dctx_fcsr(&mut self, fcsr: usize) {
        self.fp_dctx.set_fcsr(fcsr as u32)
    }
}
