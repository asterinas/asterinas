// SPDX-License-Identifier: MPL-2.0

use x86_64::registers::control::Cr0Flags;

use super::{VcpuDtable, VcpuRegs, VcpuSegment, VcpuSregs, X86GprIndex, types::VcpuMsrs};
use crate::{Error, prelude::*};

const BUSY_TSS_SEGMENT_TYPE: u8 = 0x0b;
const REAL_MODE_CODE_SEGMENT_TYPE: u8 = 0x0b;
const REAL_MODE_DATA_SEGMENT_TYPE: u8 = 0x03;
const RESET_IDT_LIMIT: u16 = 0x03ff;
const RESET_TR_SELECTOR: u16 = 0x20;

/// Stores the guest-visible architectural state of an x86 vCPU.
///
/// This context is independent of VMX hardware state. Creating or updating it
/// neither allocates a VMCS nor executes VMX instructions.
pub struct GuestContext {
    /// The guest architectural state.
    arch: VcpuArchState,

    /// The vCPU run state.
    run: VcpuRunState,
}

pub(crate) struct VcpuArchState {
    regs: VcpuRegs,
    sregs: VcpuSregs,
    msrs: VcpuMsrs,
}

impl GuestContext {
    /// Creates a guest vCPU context.
    ///
    /// The bootstrap vCPU, whose ID is zero, starts runnable. Other vCPUs wait
    /// for a startup IPI before becoming runnable.
    pub fn new(vcpu_id: u32) -> Self {
        Self {
            arch: VcpuArchState::default(),
            run: if vcpu_id == 0 {
                VcpuRunState::Runnable
            } else {
                VcpuRunState::WaitForSipi
            },
        }
    }

    /// Moves an AP vCPU from wait-for-SIPI state to runnable state.
    ///
    /// The startup vector rebuilds the vCPU's real-mode startup state. Calling
    /// this method for a vCPU that is not waiting for SIPI has no effect.
    pub fn receive_sipi(&mut self, vector: u8) {
        if self.run != VcpuRunState::WaitForSipi {
            return;
        }

        let apic_base = self.arch.sregs.apic_base;
        self.arch.regs = VcpuRegs {
            rflags: 0x2,
            ..VcpuRegs::default()
        };
        let mut sregs = VcpuSregs::with_startup(vector);
        sregs.apic_base = apic_base;
        self.arch.set_sregs(sregs);
        self.run = VcpuRunState::Runnable;
    }

    /// Gets guest's general-purpose registers.
    pub fn regs(&self) -> VcpuRegs {
        self.arch.regs
    }

    /// Replaces guest's general-purpose registers.
    pub fn set_regs(&mut self, regs: VcpuRegs) {
        self.arch.regs = regs;
    }

    /// Gets the value of a guest general-purpose register.
    pub fn gpr(&self, reg: X86GprIndex) -> u64 {
        self.arch.gpr(reg)
    }

    /// Updates the low `width_bytes` of a guest general-purpose register.
    ///
    /// One- and two-byte writes preserve the upper bits. Four-byte writes
    /// zero-extend to 64 bits, as required by the x86-64 architecture.
    pub fn set_gpr(&mut self, reg: X86GprIndex, width_bytes: u8, value: u64) -> Result<()> {
        self.arch.set_gpr(reg, width_bytes, value)
    }

    /// Advances the guest instruction pointer.
    ///
    /// Returns [`Error::Overflow`] without changing the instruction pointer if
    /// the addition overflows.
    pub fn advance_rip(&mut self, len: usize) -> Result<()> {
        self.arch.advance_rip(len)
    }

    /// Gets the guest instruction pointer.
    pub fn rip(&self) -> usize {
        self.arch.rip()
    }

    /// Gets the guest special-register state.
    pub fn sregs(&self) -> VcpuSregs {
        self.arch.sregs
    }

    /// Replaces the guest special-register state.
    ///
    /// This method stores guest-visible values without applying VMX fixed-bit
    /// adjustments or validating whether they can be loaded by hardware.
    pub fn set_sregs(&mut self, sregs: VcpuSregs) {
        self.arch.set_sregs(sregs);
    }

    /// Returns the guest-visible value of a supported MSR.
    pub fn read_msr(&self, index: u32) -> Result<u64> {
        self.arch.try_msr(index).ok_or(Error::InvalidArgs)
    }

    /// Sets the guest-visible value of a supported MSR.
    ///
    /// Unsupported indexes return [`Error::InvalidArgs`] without changing the
    /// context.
    pub fn write_msr(&mut self, index: u32, value: u64) -> Result<()> {
        if !self.arch.set_msr(index, value) {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }

    /// Gets the vCPU run state.
    pub fn run_state(&self) -> VcpuRunState {
        self.run
    }

    /// Sets the vCPU run state.
    pub fn set_run_state(&mut self, state: VcpuRunState) {
        self.run = state;
    }

    /// Returns whether the vCPU is currently running.
    pub fn is_running(&self) -> bool {
        self.run == VcpuRunState::Running
    }
}

impl Default for VcpuArchState {
    fn default() -> Self {
        Self {
            regs: VcpuRegs {
                rflags: 0x2,
                ..VcpuRegs::default()
            },
            sregs: VcpuSregs::with_startup(0),
            msrs: VcpuMsrs::default(),
        }
    }
}

impl VcpuArchState {
    fn gpr(&self, reg: X86GprIndex) -> u64 {
        (match reg {
            X86GprIndex::Rax => self.regs.rax,
            X86GprIndex::Rbx => self.regs.rbx,
            X86GprIndex::Rcx => self.regs.rcx,
            X86GprIndex::Rdx => self.regs.rdx,
            X86GprIndex::Rsi => self.regs.rsi,
            X86GprIndex::Rdi => self.regs.rdi,
            X86GprIndex::Rbp => self.regs.rbp,
            X86GprIndex::Rsp => self.regs.rsp,
            X86GprIndex::R8 => self.regs.r8,
            X86GprIndex::R9 => self.regs.r9,
            X86GprIndex::R10 => self.regs.r10,
            X86GprIndex::R11 => self.regs.r11,
            X86GprIndex::R12 => self.regs.r12,
            X86GprIndex::R13 => self.regs.r13,
            X86GprIndex::R14 => self.regs.r14,
            X86GprIndex::R15 => self.regs.r15,
        }) as u64
    }

    fn set_gpr(&mut self, reg: X86GprIndex, width_bytes: u8, value: u64) -> Result<()> {
        let slot = match reg {
            X86GprIndex::Rax => &mut self.regs.rax,
            X86GprIndex::Rbx => &mut self.regs.rbx,
            X86GprIndex::Rcx => &mut self.regs.rcx,
            X86GprIndex::Rdx => &mut self.regs.rdx,
            X86GprIndex::Rsi => &mut self.regs.rsi,
            X86GprIndex::Rdi => &mut self.regs.rdi,
            X86GprIndex::Rbp => &mut self.regs.rbp,
            X86GprIndex::Rsp => &mut self.regs.rsp,
            X86GprIndex::R8 => &mut self.regs.r8,
            X86GprIndex::R9 => &mut self.regs.r9,
            X86GprIndex::R10 => &mut self.regs.r10,
            X86GprIndex::R11 => &mut self.regs.r11,
            X86GprIndex::R12 => &mut self.regs.r12,
            X86GprIndex::R13 => &mut self.regs.r13,
            X86GprIndex::R14 => &mut self.regs.r14,
            X86GprIndex::R15 => &mut self.regs.r15,
        };
        let old_value = *slot as u64;
        let new_value = match width_bytes {
            1 => (old_value & !0xff) | (value & 0xff),
            2 => (old_value & !0xffff) | (value & 0xffff),
            4 => value & 0xffff_ffff,
            8 => value,
            _ => return Err(Error::InvalidArgs),
        };

        *slot = new_value as usize;
        Ok(())
    }

    fn advance_rip(&mut self, len: usize) -> Result<()> {
        let rip = self.regs.rip.checked_add(len).ok_or(Error::Overflow)?;
        self.regs.rip = rip;
        Ok(())
    }

    fn rip(&self) -> usize {
        self.regs.rip
    }

    fn set_sregs(&mut self, sregs: VcpuSregs) {
        self.sregs = sregs;
        self.msrs.apic_base = sregs.apic_base;
        self.msrs.efer = sregs.efer;
        self.msrs.fs_base = sregs.fs.base;
        self.msrs.gs_base = sregs.gs.base;
    }

    fn try_msr(&self, index: u32) -> Option<u64> {
        use x86::msr::*;

        Some(match index {
            IA32_TSC_ADJUST => self.msrs.tsc_adjust,
            IA32_APIC_BASE => self.msrs.apic_base,
            IA32_SYSENTER_CS => self.msrs.sysenter_cs,
            IA32_SYSENTER_ESP => self.msrs.sysenter_esp,
            IA32_SYSENTER_EIP => self.msrs.sysenter_eip,
            IA32_EFER => self.msrs.efer,
            IA32_PAT => self.msrs.pat,
            IA32_FS_BASE => self.msrs.fs_base,
            IA32_GS_BASE => self.msrs.gs_base,
            IA32_KERNEL_GSBASE => self.msrs.kernel_gs_base,
            IA32_TSC_AUX => self.msrs.tsc_aux,
            IA32_STAR => self.msrs.star,
            IA32_LSTAR => self.msrs.lstar,
            IA32_CSTAR => self.msrs.cstar,
            IA32_FMASK => self.msrs.syscall_mask,
            IA32_MISC_ENABLE => self.msrs.misc_enable,
            _ => return None,
        })
    }

    fn set_msr(&mut self, index: u32, value: u64) -> bool {
        use x86::msr::*;

        match index {
            IA32_TSC_ADJUST => self.msrs.tsc_adjust = value,
            IA32_APIC_BASE => {
                self.msrs.apic_base = value;
                self.sregs.apic_base = value;
            }
            IA32_SYSENTER_CS => self.msrs.sysenter_cs = value,
            IA32_SYSENTER_ESP => self.msrs.sysenter_esp = value,
            IA32_SYSENTER_EIP => self.msrs.sysenter_eip = value,
            IA32_EFER => {
                self.msrs.efer = value;
                self.sregs.efer = value;
            }
            IA32_PAT => self.msrs.pat = value,
            IA32_KERNEL_GSBASE => self.msrs.kernel_gs_base = value,
            IA32_TSC_AUX => self.msrs.tsc_aux = value,
            IA32_STAR => self.msrs.star = value,
            IA32_LSTAR => self.msrs.lstar = value,
            IA32_CSTAR => self.msrs.cstar = value,
            IA32_FMASK => self.msrs.syscall_mask = value,
            IA32_FS_BASE => {
                self.msrs.fs_base = value;
                self.sregs.fs.base = value;
            }
            IA32_GS_BASE => {
                self.msrs.gs_base = value;
                self.sregs.gs.base = value;
            }
            IA32_MISC_ENABLE => self.msrs.misc_enable = value,
            _ => return false,
        }

        true
    }
}

impl Default for GuestContext {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Describes whether a guest vCPU can enter guest mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VcpuRunState {
    /// The vCPU is waiting for a startup IPI.
    WaitForSipi,
    /// The vCPU is ready to enter guest mode.
    Runnable,
    /// The vCPU is executing in guest mode.
    Running,
    /// The vCPU is halted until an event makes it runnable.
    Halted,
}

impl VcpuSregs {
    fn with_startup(startup_vector: u8) -> Self {
        let code_base = u64::from(startup_vector) << 12;
        let code_selector = u16::from(startup_vector) << 8;
        let data = real_mode_segment(0, 0, REAL_MODE_DATA_SEGMENT_TYPE, 1);

        Self {
            cs: real_mode_segment(code_selector, code_base, REAL_MODE_CODE_SEGMENT_TYPE, 1),
            ds: data,
            es: data,
            fs: data,
            gs: data,
            ss: data,
            tr: real_mode_segment(RESET_TR_SELECTOR, 0, BUSY_TSS_SEGMENT_TYPE, 0),
            ldt: VcpuSegment {
                unusable: 1,
                ..VcpuSegment::default()
            },
            idt: VcpuDtable {
                base: 0,
                limit: RESET_IDT_LIMIT,
                padding: [0; 3],
            },
            cr0: (Cr0Flags::EXTENSION_TYPE | Cr0Flags::NUMERIC_ERROR).bits(),
            ..VcpuSregs::default()
        }
    }
}

fn real_mode_segment(selector: u16, base: u64, type_: u8, s: u8) -> VcpuSegment {
    VcpuSegment {
        base,
        limit: 0xffff,
        selector,
        type_,
        present: 1,
        dpl: 0,
        db: 0,
        s,
        l: 0,
        g: 0,
        avl: 0,
        unusable: 0,
        padding: 0,
    }
}
