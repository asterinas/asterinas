// SPDX-License-Identifier: MPL-2.0

//! RISC-V ptrace ABI.

use core::mem::offset_of;

use ostd::arch::cpu::context::{FpuContext, UserContext};

use super::cpu::SigContext;
use crate::{prelude::*, process::posix_thread::SuppUserContext};

#[repr(transparent)]
#[derive(Clone, Copy, Pod)]
struct CUserRegsStruct {
    regs: SigContext,
}

impl CUserRegsStruct {
    fn from_context(user_context: &UserContext) -> Self {
        let mut regs = SigContext::default();
        regs.copy_user_regs_from(user_context);
        Self { regs }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct CUserFpregsStruct {
    f: [u64; 32],
    fcsr: u32,
    // Linux pads the floating-point regset to an 8-byte boundary and returns the padding as zero.
    // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/riscv/kernel/ptrace.c#L67>
    padding: u32,
}

impl CUserFpregsStruct {
    const PAYLOAD_LEN: usize = offset_of!(Self, padding);

    fn from_context(context: &FpuContext) -> Result<Self> {
        match context {
            FpuContext::F(_) => Err(errno!(EINVAL, "unsupported F-only floating-point context")),
            FpuContext::D(state) => Ok(Self {
                padding: 0,
                ..Self::from_bytes(state.as_bytes())
            }),
            // TODO: Support reading Q state. Linux uses FSD, so its D-format regset
            // exposes only the low 64 bits of each 128-bit register.
            FpuContext::Q(_) => Err(errno!(EINVAL, "unsupported Q floating-point context")),
            FpuContext::None => Err(errno!(EINVAL, "floating-point registers are unavailable")),
        }
    }

    /// Writes the floating-point registers and `fcsr`, preserving the padding bytes in the context.
    fn apply_to(&self, context: &mut FpuContext) -> Result<()> {
        match context {
            FpuContext::F(_) => Err(errno!(EINVAL, "unsupported F-only floating-point context")),
            FpuContext::D(state) => {
                state.as_mut_bytes()[..Self::PAYLOAD_LEN]
                    .copy_from_slice(&self.as_bytes()[..Self::PAYLOAD_LEN]);
                Ok(())
            }
            // TODO: Support writing Q state. Linux restores registers with FLD,
            // which fills the upper 64 bits with ones on Q hardware.
            FpuContext::Q(_) => Err(errno!(EINVAL, "unsupported Q floating-point context")),
            FpuContext::None => Err(errno!(EINVAL, "floating-point registers are unavailable")),
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum Regset {
    /// General-purpose registers (`NT_PRSTATUS`).
    General = 1,
    /// Floating-point registers (`NT_PRFPREG`).
    Fp = 2,
}

impl Regset {
    /// Validates the requested byte length and truncates it to the register set size.
    pub(crate) fn validate_and_truncate_len(self, len: usize) -> Result<usize> {
        let size = match self {
            Self::General => size_of::<CUserRegsStruct>(),
            Self::Fp => size_of::<CUserFpregsStruct>(),
        };

        if !len.is_multiple_of(size_of::<usize>()) {
            return Err(errno!(EINVAL, "invalid register set length"));
        }
        Ok(len.min(size))
    }
}

/// RISC-V register state saved while a tracee is stopped.
pub(crate) struct PtraceState {
    user_context: UserContext,
    orig_syscall_ret: Option<usize>,
    fpu_context: FpuContext,
    is_fpu_dirty: bool,
}

impl PtraceState {
    pub(crate) fn new(
        user_context: &UserContext,
        supp_context: &SuppUserContext,
        orig_syscall_ret: Option<usize>,
    ) -> Self {
        Self {
            user_context: user_context.clone(),
            orig_syscall_ret,
            fpu_context: supp_context.fpu().get(),
            is_fpu_dirty: false,
        }
    }

    pub(crate) fn restore(self, user_context: &mut UserContext, supp_context: &SuppUserContext) {
        *user_context = self.user_context;
        if self.is_fpu_dirty {
            supp_context.fpu().set(self.fpu_context);
        }
    }

    pub(crate) fn orig_syscall_ret(&self) -> Option<usize> {
        self.orig_syscall_ret
    }

    pub(crate) fn get_regset(&self, regset: Regset, writer: &mut VmWriter) -> Result<()> {
        match regset {
            Regset::General => {
                let regs = CUserRegsStruct::from_context(&self.user_context);
                writer.write_fallible(&mut VmReader::from(regs.as_bytes()))?;
            }
            Regset::Fp => {
                let regs = CUserFpregsStruct::from_context(&self.fpu_context)?;
                writer.write_fallible(&mut VmReader::from(regs.as_bytes()))?;
            }
        }
        Ok(())
    }

    pub(crate) fn set_regset(&mut self, regset: Regset, reader: &mut VmReader) -> Result<()> {
        match regset {
            Regset::General => {
                if !reader.has_remain() {
                    return Ok(());
                }
                let mut general = CUserRegsStruct::from_context(&self.user_context);
                reader.read_fallible(&mut VmWriter::from(general.as_mut_bytes()))?;
                general.regs.copy_user_regs_to(&mut self.user_context);
                Ok(())
            }
            Regset::Fp => {
                let mut regs = CUserFpregsStruct::from_context(&self.fpu_context)?;
                if !reader.has_remain() {
                    return Ok(());
                }
                reader.read_fallible(&mut VmWriter::from(
                    &mut regs.as_mut_bytes()[..CUserFpregsStruct::PAYLOAD_LEN],
                ))?;
                regs.apply_to(&mut self.fpu_context)?;
                self.is_fpu_dirty = true;
                Ok(())
            }
        }
    }
}
