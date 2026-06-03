// SPDX-License-Identifier: MPL-2.0

//! x86-64 ptrace ABI.

use core::ops::Range;

use ostd::{
    arch::{
        cpu::context::{FsBase, GeneralRegs, GsBase},
        trap::{USER_CS_VALUE, USER_SS_VALUE},
    },
    mm::MAX_USERSPACE_VADDR,
};
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

impl CUserRegsStruct {
    /// Builds the register snapshot from saved register state.
    ///
    /// `orig_rax` is left at zero. Callers needing the syscall-entry
    /// value should assign it from `ThreadLocal::orig_syscall_ret`.
    pub fn from_regs(general_regs: &GeneralRegs, fs_base: FsBase, gs_base: GsBase) -> Self {
        let mut out = Self::default();
        let bytes = out.as_mut_bytes();
        for rule in REG_RULES {
            let value = match rule.policy {
                Policy::Fixed(val) => val,
                _ => (rule.get.unwrap())(general_regs),
            };
            write_word(bytes, rule.offset, value);
        }
        write_word(
            bytes,
            core::mem::offset_of!(CUserRegsStruct, fsbase),
            fs_base.addr(),
        );
        write_word(
            bytes,
            core::mem::offset_of!(CUserRegsStruct, gsbase),
            gs_base.addr(),
        );
        out
    }

    /// Validates the user-supplied `regs` and then applies it into saved register state.
    ///
    /// `orig_rax` is ignored. Callers should separately assign this
    /// field to `ThreadLocal::orig_syscall_ret`.
    ///
    /// # Errors
    ///
    /// Returns `EIO` on any invalid value.
    pub fn apply_to(
        &self,
        general_regs: &mut GeneralRegs,
        fs_base: &mut FsBase,
        gs_base: &mut GsBase,
    ) -> Result<()> {
        let bytes = self.as_bytes();
        let mut new_general_regs = *general_regs;
        for rule in REG_RULES {
            let value = read_word(bytes, rule.offset);
            rule.apply(&mut new_general_regs, value)?;
        }

        let fsbase = read_word(bytes, core::mem::offset_of!(CUserRegsStruct, fsbase));
        if !is_user_addr(fsbase) {
            return_errno_with_message!(Errno::EIO, "invalid register value");
        }

        let gsbase = read_word(bytes, core::mem::offset_of!(CUserRegsStruct, gsbase));
        if !is_user_addr(gsbase) {
            return_errno_with_message!(Errno::EIO, "invalid register value");
        }

        *general_regs = new_general_regs;
        *fs_base = FsBase::new(fsbase);
        *gs_base = GsBase::new(gsbase);
        Ok(())
    }
}

/// Reads one word from the x86-64 USER area at `offset`.
pub fn read_user_word(
    general_regs: &GeneralRegs,
    fs_base: FsBase,
    gs_base: GsBase,
    orig_rax: usize,
    offset: usize,
) -> Result<usize> {
    check_user_offset(offset)?;
    if offset == core::mem::offset_of!(CUserRegsStruct, fsbase) {
        return Ok(fs_base.addr());
    }
    if offset == core::mem::offset_of!(CUserRegsStruct, gsbase) {
        return Ok(gs_base.addr());
    }
    if offset == core::mem::offset_of!(CUserRegsStruct, orig_rax) {
        return Ok(orig_rax);
    }

    // FIXME: This emulates the default state of the x86 debug registers,
    // so it can correctly respond when a tracer reads the tracee’s debug
    // registers via `PTRACE_PEEKUSER`.
    // Currently, the tracee’s x86 debug registers are never actually modified,
    // so they always remain at their default values.
    if let Some(index) = debug_register_index(offset) {
        let value = match index {
            6 => DEBUG_STATUS_DEFAULT_VALUE,
            0..=5 | 7 => 0,
            _ => unreachable!(),
        };
        return Ok(value);
    }

    let rule =
        RegRule::for_offset(offset).expect("offset has been validated by `check_user_offset`");
    Ok(match rule.policy {
        Policy::Fixed(value) => value,
        _ => (rule.get.unwrap())(general_regs),
    })
}

/// Writes one word to the x86-64 USER area at `offset`.
pub fn write_user_word(
    general_regs: &mut GeneralRegs,
    fs_base: &mut FsBase,
    gs_base: &mut GsBase,
    orig_rax: &mut usize,
    offset: usize,
    value: usize,
) -> Result<()> {
    check_user_offset(offset)?;
    if offset == core::mem::offset_of!(CUserRegsStruct, fsbase) {
        if !is_user_addr(value) {
            return_errno_with_message!(Errno::EIO, "invalid register value");
        }
        *fs_base = FsBase::new(value);
        return Ok(());
    }
    if offset == core::mem::offset_of!(CUserRegsStruct, gsbase) {
        if !is_user_addr(value) {
            return_errno_with_message!(Errno::EIO, "invalid register value");
        }
        *gs_base = GsBase::new(value);
        return Ok(());
    }
    if offset == core::mem::offset_of!(CUserRegsStruct, orig_rax) {
        *orig_rax = value;
        return Ok(());
    }
    if debug_register_index(offset).is_some() {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing x86 debug registers is not supported currently"
        );
    }

    let rule =
        RegRule::for_offset(offset).expect("offset has been validated by `check_user_offset`");
    rule.apply(general_regs, value)
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
/// `orig_rax`, `fsbase`, and `gsbase` are handled separately because they are
/// not stored in `GeneralRegs`.
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
    assert!((REG_RULES.len() + 3) * size_of::<usize>() == size_of::<CUserRegsStruct>());
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
    if !offset.is_multiple_of(size_of::<usize>()) {
        return_errno_with_message!(Errno::EIO, "invalid USER area offset");
    }

    // We only support the offsets for general-purpose registers,
    // and x86 debug registers currently.
    // `struct user_regs_struct` is the first field in `struct user`.
    if offset < size_of::<CUserRegsStruct>() {
        return Ok(());
    }

    if debug_register_index(offset).is_some() {
        return Ok(());
    }

    return_errno_with_message!(
        Errno::EOPNOTSUPP,
        "only offsets for general-purpose registers and debug registers are supported currently"
    );
}

// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/include/asm/user_64.h#L103-L132>.
const DEBUG_REGS_OFFSET: usize = 848;
const DEBUG_REGS_COUNT: usize = 8;
const DEBUG_STATUS_DEFAULT_VALUE: usize = 0xffff0ff0;

const fn debug_register_index(offset: usize) -> Option<usize> {
    if !offset.is_multiple_of(size_of::<usize>()) {
        return None;
    }

    if offset < DEBUG_REGS_OFFSET
        || offset >= DEBUG_REGS_OFFSET + DEBUG_REGS_COUNT * size_of::<usize>()
    {
        return None;
    }

    Some((offset - DEBUG_REGS_OFFSET) / size_of::<usize>())
}

fn read_word(bytes: &[u8], offset: usize) -> usize {
    usize::from_ne_bytes(bytes[word_range(offset)].try_into().unwrap())
}

fn write_word(bytes: &mut [u8], offset: usize, value: usize) {
    bytes[word_range(offset)].copy_from_slice(&value.to_ne_bytes());
}

const fn word_range(offset: usize) -> Range<usize> {
    offset..offset + size_of::<usize>()
}

// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/include/asm/segment.h#L188-L189>
const LINUX_USER_CS: usize = 0x33;
const LINUX_USER_SS: usize = 0x2b;

// Some applications (e.g., GDB) rely on `ptrace` exposing Linux's
// conventional x86-64 user segment selector values.
//
// See: <https://sourceware.org/git/?p=binutils-gdb.git;a=blob;f=gdb/nat/x86-linux.c;h=16391dcde24407fc2c219194bc960f761bd048fe;hb=HEAD#l124>.
ostd::const_assert!(LINUX_USER_CS == USER_CS_VALUE);
ostd::const_assert!(LINUX_USER_SS == USER_SS_VALUE);
