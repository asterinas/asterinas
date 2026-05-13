// SPDX-License-Identifier: MPL-2.0

//! x86-64 ptrace ABI.

use core::ops::Range;

use ostd::{
    arch::{
        cpu::context::GeneralRegs,
        trap::{USER_CS_VALUE, USER_SS_VALUE},
    },
    mm::MAX_USERSPACE_VADDR,
};
use ostd_pod::IntoBytes;
use x86_64::registers::rflags::RFlags;

use crate::prelude::*;

// =====================================================================
// Public ABI mirror.
// =====================================================================

/// Mirror of Linux's `struct user_regs_struct` for x86-64.
//
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/include/asm/user_64.h#L66-L97>
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct CUserRegsStruct {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub rbp: usize,
    pub rbx: usize,
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rax: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub orig_rax: usize,
    pub rip: usize,
    pub cs: usize,
    pub rflags: usize,
    pub rsp: usize,
    pub ss: usize,
    pub fsbase: usize,
    pub gsbase: usize,
    pub ds: usize,
    pub es: usize,
    pub fs: usize,
    pub gs: usize,
}

impl From<&GeneralRegs> for CUserRegsStruct {
    /// Builds the register snapshot from saved general-purpose registers.
    ///
    /// `orig_rax` is left at zero. Callers needing the syscall-entry
    /// value should assign it from `PosixThread::orig_syscall_ret`.
    fn from(regs: &GeneralRegs) -> Self {
        let mut out = Self::default();
        let bytes = out.as_mut_bytes();
        for rule in REG_RULES {
            let value = match rule.policy {
                Policy::Fixed(val) => val,
                _ => (rule.get.unwrap())(regs),
            };
            write_word(bytes, rule.offset, value);
        }
        out
    }
}

impl CUserRegsStruct {
    /// Validates the user-supplied `regs` and then applies it into `regs`.
    ///
    /// `orig_rax` is ignored. Callers should separately assign this
    /// field to `PosixThread::orig_syscall_ret`.
    ///
    /// # Errors
    ///
    /// Returns `EIO` on any invalid value.
    pub fn apply_to(&self, regs: &mut GeneralRegs) -> Result<()> {
        let bytes = self.as_bytes();
        let mut new_regs = *regs;
        for rule in REG_RULES {
            let value = read_word(bytes, rule.offset);
            rule.apply(&mut new_regs, value)?;
        }
        *regs = new_regs;
        Ok(())
    }
}

/// Reads one word from the x86-64 USER area at `offset`.
pub fn read_user_word(regs: &GeneralRegs, orig_rax: usize, offset: usize) -> Result<usize> {
    check_user_offset(offset)?;
    if offset == core::mem::offset_of!(CUserRegsStruct, orig_rax) {
        return Ok(orig_rax);
    }

    let rule =
        RegRule::for_offset(offset).expect("offset has been validated by `check_user_offset`");
    Ok(match rule.policy {
        Policy::Fixed(value) => value,
        _ => (rule.get.unwrap())(regs),
    })
}

/// Writes one word to the x86-64 USER area at `offset`.
pub fn write_user_word(
    regs: &mut GeneralRegs,
    orig_rax: &mut usize,
    offset: usize,
    value: usize,
) -> Result<()> {
    check_user_offset(offset)?;
    if offset == core::mem::offset_of!(CUserRegsStruct, orig_rax) {
        *orig_rax = value;
        return Ok(());
    }

    let rule =
        RegRule::for_offset(offset).expect("offset has been validated by `check_user_offset`");
    rule.apply(regs, value)
}

/// Enables x86-64 single-step execution by setting the trap flag.
pub fn enable_single_step(regs: &mut GeneralRegs) {
    regs.rflags |= RFlags::TRAP_FLAG.bits() as usize;
}

/// Disables x86-64 single-step execution by clearing the trap flag.
pub fn disable_single_step(regs: &mut GeneralRegs) {
    regs.rflags &= !(RFlags::TRAP_FLAG.bits() as usize);
}

// =====================================================================
// Per-register policy table.
// =====================================================================

macro_rules! off {
    ($field:ident) => {
        core::mem::offset_of!(CUserRegsStruct, $field)
    };
}

/// Defines the ptrace access rules for `CUserRegsStruct`.
///
/// Each entry covers one `usize` field in `CUserRegsStruct` that is either
/// backed by `GeneralRegs` or fixed to an ABI-defined value.
/// `orig_rax` is the only exception: it is stored in
/// `PosixThread::orig_syscall_ret` rather than `GeneralRegs`.
const REG_RULES: &[RegRule] = &[
    RegRule::rw(off!(rax), |r| r.rax, |r, v| r.rax = v, Policy::Set),
    RegRule::rw(off!(rbx), |r| r.rbx, |r, v| r.rbx = v, Policy::Set),
    RegRule::rw(off!(rcx), |r| r.rcx, |r, v| r.rcx = v, Policy::Set),
    RegRule::rw(off!(rdx), |r| r.rdx, |r, v| r.rdx = v, Policy::Set),
    RegRule::rw(off!(rsi), |r| r.rsi, |r, v| r.rsi = v, Policy::Set),
    RegRule::rw(off!(rdi), |r| r.rdi, |r, v| r.rdi = v, Policy::Set),
    RegRule::rw(off!(rbp), |r| r.rbp, |r, v| r.rbp = v, Policy::Set),
    RegRule::rw(off!(r8), |r| r.r8, |r, v| r.r8 = v, Policy::Set),
    RegRule::rw(off!(r9), |r| r.r9, |r, v| r.r9 = v, Policy::Set),
    RegRule::rw(off!(r10), |r| r.r10, |r, v| r.r10 = v, Policy::Set),
    RegRule::rw(off!(r11), |r| r.r11, |r, v| r.r11 = v, Policy::Set),
    RegRule::rw(off!(r12), |r| r.r12, |r, v| r.r12 = v, Policy::Set),
    RegRule::rw(off!(r13), |r| r.r13, |r, v| r.r13 = v, Policy::Set),
    RegRule::rw(off!(r14), |r| r.r14, |r, v| r.r14 = v, Policy::Set),
    RegRule::rw(off!(r15), |r| r.r15, |r, v| r.r15 = v, Policy::Set),
    // These are more strict than Linux.
    RegRule::rw(
        off!(rsp),
        |r| r.rsp,
        |r, v| r.rsp = v,
        Policy::SetIf(is_user_addr),
    ),
    RegRule::rw(
        off!(rip),
        |r| r.rip,
        |r, v| r.rip = v,
        Policy::SetIf(is_user_addr),
    ),
    // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/kernel/ptrace.c#L389-L400>
    RegRule::rw(
        off!(fsbase),
        |r| r.fsbase,
        |r, v| r.fsbase = v,
        Policy::SetIf(is_user_addr),
    ),
    RegRule::rw(
        off!(gsbase),
        |r| r.gsbase,
        |r, v| r.gsbase = v,
        Policy::SetIf(is_user_addr),
    ),
    // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/kernel/ptrace.c#L369>
    RegRule::rw(
        off!(rflags),
        |r| r.rflags,
        |r, v| r.rflags = v,
        Policy::SetBitsTruncate(USER_MODIFIABLE_RFLAGS_MASK),
    ),
    // Asterinas does not run 32-bit user code,
    // so the segment selectors are always at fixed values.
    RegRule::fixed(off!(cs), USER_CS_VALUE),
    RegRule::fixed(off!(ss), USER_SS_VALUE),
    RegRule::fixed(off!(ds), 0),
    RegRule::fixed(off!(es), 0),
    RegRule::fixed(off!(fs), 0),
    RegRule::fixed(off!(gs), 0),
];

const _: () = {
    assert!(
        (REG_RULES.len() + 1) * core::mem::size_of::<usize>()
            == core::mem::size_of::<CUserRegsStruct>()
    );
};

/// One rule for a single register inside `CUserRegsStruct`.
struct RegRule {
    /// Byte offset of this register inside `CUserRegsStruct`.
    offset: usize,
    /// Read accessor on `GeneralRegs`, `None` for `Policy::Fixed`.
    get: Option<fn(&GeneralRegs) -> usize>,
    /// Write accessor on `GeneralRegs`, `None` for `Policy::Fixed`.
    set: Option<fn(&mut GeneralRegs, usize)>,
    /// Policy for validating user input when writing this register.
    policy: Policy,
}

impl RegRule {
    /// Builds a rule for a `GeneralRegs`-backed register.
    const fn rw(
        offset: usize,
        get: fn(&GeneralRegs) -> usize,
        set: fn(&mut GeneralRegs, usize),
        policy: Policy,
    ) -> Self {
        Self {
            offset,
            get: Some(get),
            set: Some(set),
            policy,
        }
    }

    /// Builds a rule for a pseudo field fixed at `expected`.
    const fn fixed(offset: usize, expected: usize) -> Self {
        Self {
            offset,
            get: None,
            set: None,
            policy: Policy::Fixed(expected),
        }
    }

    /// Returns the rule whose offset matches `offset`.
    fn for_offset(offset: usize) -> Option<&'static Self> {
        REG_RULES.iter().find(|rule| rule.offset == offset)
    }

    /// Applies one user-supplied register write according to this rule.
    fn apply(&self, regs: &mut GeneralRegs, value: usize) -> Result<()> {
        match self.policy {
            Policy::Set => {
                (self.set.unwrap())(regs, value);
            }
            Policy::SetIf(check) => {
                if !check(value) {
                    return_errno_with_message!(Errno::EIO, "invalid register value");
                }
                (self.set.unwrap())(regs, value);
            }
            Policy::SetBitsTruncate(mask) => {
                let current = (self.get.unwrap())(regs);
                (self.set.unwrap())(regs, (current & !mask) | (value & mask));
            }
            Policy::Fixed(expected) => {
                if value != expected {
                    return_errno_with_message!(Errno::EIO, "invalid fixed register value");
                }
            }
        }

        Ok(())
    }
}

/// The policy for validating user input when writing a register.
enum Policy {
    /// Unrestricted.
    Set,
    /// Set only if the predicate accepts the value.
    SetIf(fn(usize) -> bool),
    /// Replace only the bits in the mask, preserve the rest.
    SetBitsTruncate(usize),
    /// The field is fixed at the expected value.
    Fixed(usize),
}

const fn is_user_addr(v: usize) -> bool {
    v < MAX_USERSPACE_VADDR
}

/// RFLAGS bits the user may modify via ptrace.
//
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/kernel/ptrace.c#L241>.
const USER_MODIFIABLE_RFLAGS_MASK: usize = (RFlags::CARRY_FLAG.bits()
    | RFlags::PARITY_FLAG.bits()
    | RFlags::AUXILIARY_CARRY_FLAG.bits()
    | RFlags::ZERO_FLAG.bits()
    | RFlags::SIGN_FLAG.bits()
    | RFlags::TRAP_FLAG.bits()
    | RFlags::DIRECTION_FLAG.bits()
    | RFlags::OVERFLOW_FLAG.bits()
    | RFlags::RESUME_FLAG.bits()
    | RFlags::ALIGNMENT_CHECK.bits()
    | RFlags::NESTED_TASK.bits()) as usize;

/// Checks whether the given offset is valid in `struct user`.
//
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/include/asm/user_64.h#L103-L132>
fn check_user_offset(offset: usize) -> Result<()> {
    // We only support the offsets for general-purpose registers currently.
    // `struct user_regs_struct` is the first field in `struct user`.
    if offset >= core::mem::size_of::<CUserRegsStruct>() {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "only offsets for general-purpose registers are supported currently"
        );
    }

    if !offset.is_multiple_of(core::mem::size_of::<usize>()) {
        return_errno_with_message!(Errno::EIO, "invalid USER area offset");
    }

    Ok(())
}

fn read_word(bytes: &[u8], offset: usize) -> usize {
    usize::from_ne_bytes(bytes[word_range(offset)].try_into().unwrap())
}

fn write_word(bytes: &mut [u8], offset: usize, value: usize) {
    bytes[word_range(offset)].copy_from_slice(&value.to_ne_bytes());
}

const fn word_range(offset: usize) -> Range<usize> {
    offset..offset + core::mem::size_of::<usize>()
}
