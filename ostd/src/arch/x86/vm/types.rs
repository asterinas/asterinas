// SPDX-License-Identifier: MPL-2.0

//! Types that represent guest-visible x86 CPU state.

use crate::{Error, prelude::*};

/// Guest general-purpose registers.
#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuRegs {
    pub rax: usize,
    pub rbx: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rbp: usize,
    pub rsp: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
    pub rip: usize,
    pub rflags: usize,
}

/// A guest general-purpose register.
#[expect(missing_docs)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum X86GprIndex {
    Rax,
    Rbx,
    Rcx,
    Rdx,
    Rsi,
    Rdi,
    Rbp,
    Rsp,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

impl X86GprIndex {
    /// Converts an x86 register encoding into a general-purpose register.
    ///
    /// This encoding is used by `ModR/M.reg` and by bits 11:8 of the
    /// VMX control-register-access exit qualification.
    ///
    /// See Intel SDM (June 2026), Vol. 2A, Sections 2.1.5 and
    /// Vol. 3C, Section 29.2.1, Table 29-3.
    pub fn from_x86_reg_encoding(index: u8) -> Result<Self> {
        Ok(match index {
            0 => Self::Rax,
            1 => Self::Rcx,
            2 => Self::Rdx,
            3 => Self::Rbx,
            4 => Self::Rsp,
            5 => Self::Rbp,
            6 => Self::Rsi,
            7 => Self::Rdi,
            8 => Self::R8,
            9 => Self::R9,
            10 => Self::R10,
            11 => Self::R11,
            12 => Self::R12,
            13 => Self::R13,
            14 => Self::R14,
            15 => Self::R15,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Guest MSR state that is not already stored in [`VcpuSregs`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct VcpuMsrs {
    pub pat: u64,
    pub kernel_gs_base: u64,
    pub star: u64,
    pub lstar: u64,
    pub cstar: u64,
    pub syscall_mask: u64,
    pub tsc_adjust: u64,
    pub tsc_aux: u64,
    pub sysenter_cs: u64,
    pub sysenter_esp: u64,
    pub sysenter_eip: u64,
    pub misc_enable: u64,
}

impl Default for VcpuMsrs {
    fn default() -> Self {
        Self {
            pat: 0x0007_0406_0007_0406,
            kernel_gs_base: 0,
            star: 0,
            lstar: 0,
            cstar: 0,
            syscall_mask: 0,
            tsc_adjust: 0,
            tsc_aux: 0,
            sysenter_cs: 0,
            sysenter_esp: 0,
            sysenter_eip: 0,
            misc_enable: 0,
        }
    }
}

/// Guest special-register state.
#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuSregs {
    pub cs: VcpuSegment,
    pub ds: VcpuSegment,
    pub es: VcpuSegment,
    pub fs: VcpuSegment,
    pub gs: VcpuSegment,
    pub ss: VcpuSegment,
    pub tr: VcpuSegment,
    pub ldt: VcpuSegment,
    pub gdt: VcpuDtable,
    pub idt: VcpuDtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4],
}

/// Guest segment-register state.
/// Refer to: Intel SDM Vol. 3A, 3.4.5 "Segment Descriptors".
#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuSegment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    pub padding: u8,
}

/// Guest descriptor-table state.
#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuDtable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}
