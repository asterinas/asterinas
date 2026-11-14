// SPDX-License-Identifier: MPL-2.0

//! x86-64 ptrace ABI.

use core::ops::Range;

use ostd::{
    arch::{
        cpu::context::{FpuContext, FsBase, GeneralRegs, GsBase, UserContext},
        trap::{USER_CS_VALUE, USER_SS_VALUE},
    },
    mm::MAX_USERSPACE_VADDR,
};
use spin::Once;
use x86_64::registers::rflags::RFlags;

use crate::{prelude::*, process::posix_thread::SuppUserContext};

/// The MXCSR bits supported by the processor.
static MXCSR_MASK: Once<u32> = Once::new();

pub(super) fn init() {
    MXCSR_MASK.call_once(|| {
        let mut context = FpuContext::new();
        context.save();
        let regs =
            CUserFpregsStruct::from_bytes(&context.as_bytes()[..size_of::<CUserFpregsStruct>()]);

        // Intel SDM Vol. 1, Section 11.6.6: a zero MXCSR_MASK denotes the
        // architectural fallback mask (which excludes DAZ).
        if regs.mxcsr_mask == 0 {
            0x0000_ffbf
        } else {
            regs.mxcsr_mask
        }
    });
}

// =====================================================================
// Public ABI mirror.
// =====================================================================

/// Mirror of Linux's `struct user_regs_struct` for x86-64.
//
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/include/asm/user_64.h#L66-L97>
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct CUserRegsStruct {
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
    fn from_regs(user_context: &UserContext, fs_base: FsBase, gs_base: GsBase) -> Self {
        let mut out = Self::default();
        let bytes = out.as_mut_bytes();
        for rule in REG_RULES {
            let value = match rule.policy {
                Policy::Fixed(val) => val,
                _ => (rule.get.unwrap())(user_context.general_regs()),
            };
            write_word(bytes, rule.offset, value);
        }
        write_word(
            bytes,
            core::mem::offset_of!(CUserRegsStruct, rflags),
            user_context.rflags(),
        );
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

    /// Validates and applies only the registers in the supplied byte prefix.
    ///
    /// `orig_rax` is ignored. Callers should separately assign this
    /// field to `ThreadLocal::orig_syscall_ret`.
    ///
    /// # Errors
    ///
    /// Returns `EIO` on any invalid value.
    fn apply_prefix_to(
        &self,
        user_context: &mut UserContext,
        fs_base: &mut FsBase,
        gs_base: &mut GsBase,
        len: usize,
    ) -> Result<()> {
        let bytes = self.as_bytes();
        debug_assert!(len <= bytes.len() && len.is_multiple_of(size_of::<usize>()));

        let mut new_general_regs = *user_context.general_regs();
        for rule in REG_RULES.iter().filter(|rule| rule.offset < len) {
            let value = read_word(bytes, rule.offset);
            rule.apply(&mut new_general_regs, value)?;
        }

        let should_set_fsbase = core::mem::offset_of!(CUserRegsStruct, fsbase) < len;
        if should_set_fsbase && !is_user_addr(self.fsbase) {
            return_errno_with_message!(Errno::EIO, "invalid register value");
        }

        let should_set_gsbase = core::mem::offset_of!(CUserRegsStruct, gsbase) < len;
        if should_set_gsbase && !is_user_addr(self.gsbase) {
            return_errno_with_message!(Errno::EIO, "invalid register value");
        }

        *user_context.general_regs_mut() = new_general_regs;
        if core::mem::offset_of!(CUserRegsStruct, rflags) < len {
            user_context.set_rflags(self.rflags);
        }
        if should_set_fsbase {
            *fs_base = FsBase::new(self.fsbase);
        }
        if should_set_gsbase {
            *gs_base = GsBase::new(self.gsbase);
        }
        Ok(())
    }
}

// State component masks in the XSAVE header's XSTATE_BV field.
//
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/include/asm/fpu/types.h#L133-L134>
const XFEATURE_MASK_FP: u64 = 1 << 0;
const XFEATURE_MASK_SSE: u64 = 1 << 1;

/// Mirror of Linux's `struct user_i387_struct` for x86-64.
//
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/include/asm/user_64.h#L48-L64>
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct CUserFpregsStruct {
    cwd: u16,
    swd: u16,
    ftw: u16,
    fop: u16,
    rip: u64,
    rdp: u64,
    mxcsr: u32,
    mxcsr_mask: u32,
    st_space: [u32; 32],
    xmm_space: [u32; 64],
    padding: [u32; 24],
}

impl CUserFpregsStruct {
    const XSTATE_BV_OFFSET: usize = 512;

    /// Builds the ABI snapshot from saved FPU state.
    fn from_context(context: &FpuContext) -> Self {
        let bytes = context.as_bytes();
        let mut out = Self::from_bytes(&bytes[..size_of::<Self>()]);

        // Legacy FXSAVE contexts have no header or initial-state bits.
        if let Some(xstate_bv) =
            bytes.get(Self::XSTATE_BV_OFFSET..Self::XSTATE_BV_OFFSET + size_of::<u64>())
        {
            let features = u64::from_bytes(xstate_bv);

            // XRSTOR initializes components whose state bits are clear.
            if features & XFEATURE_MASK_FP == 0 {
                out.cwd = 0x037f;
                out.swd = 0;
                // The high byte is reserved and is preserved on reads.
                out.ftw &= 0xff00;
                out.fop = 0;
                out.rip = 0;
                out.rdp = 0;
                out.st_space.fill(0);
            }
            if features & XFEATURE_MASK_SSE == 0 {
                out.xmm_space.fill(0);
            }
            out.padding.fill(0);
        }

        out
    }

    /// Validates and applies the x87/SSE state, preserving extended components.
    fn apply_to(&self, context: &mut FpuContext) -> Result<()> {
        if self.mxcsr & !*MXCSR_MASK.get().unwrap() != 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid MXCSR value");
        }

        let bytes = context.as_bytes_mut();
        bytes[..size_of::<Self>()].copy_from_slice(self.as_bytes());

        // FXRSTOR restores the legacy area without consulting a header.
        if let Some(xstate_bv) =
            bytes.get_mut(Self::XSTATE_BV_OFFSET..Self::XSTATE_BV_OFFSET + size_of::<u64>())
        {
            // XRSTOR otherwise ignores writes to initial-state components.
            let features = u64::from_bytes(xstate_bv) | XFEATURE_MASK_FP | XFEATURE_MASK_SSE;
            xstate_bv.copy_from_slice(features.as_bytes());
        }
        Ok(())
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum Regset {
    /// General-purpose registers (`NT_PRSTATUS`).
    General = 1,
    /// x87 and SSE registers (`NT_PRFPREG`).
    Fp = 2,
}

impl Regset {
    /// Validates the requested byte length and truncates it to the register set size.
    pub(crate) fn transfer_len(self, len: usize) -> Result<usize> {
        let (size, element_size) = match self {
            Self::General => (size_of::<CUserRegsStruct>(), size_of::<usize>()),
            Self::Fp => (size_of::<CUserFpregsStruct>(), size_of::<usize>()),
        };

        if !len.is_multiple_of(element_size) {
            return_errno_with_message!(Errno::EINVAL, "invalid register set length");
        }
        Ok(len.min(size))
    }
}

/// x86-64 register state saved while a tracee is stopped.
pub(crate) struct PtraceState {
    user_context: UserContext,
    fs_base: FsBase,
    gs_base: GsBase,
    orig_syscall_ret: usize,
    fpu_context: FpuContext,
    is_fpu_dirty: bool,
}

impl PtraceState {
    const NOT_A_SYSCALL: usize = usize::MAX;

    /// Creates the x86-64 ptrace state from saved registers.
    pub(crate) fn new(
        user_context: &UserContext,
        supp_context: &SuppUserContext,
        orig_syscall_ret: Option<usize>,
    ) -> Self {
        Self {
            user_context: user_context.clone(),
            fs_base: supp_context.fs_base().get(),
            gs_base: supp_context.gs_base().get(),
            orig_syscall_ret: orig_syscall_ret.unwrap_or(Self::NOT_A_SYSCALL),
            fpu_context: supp_context.fpu().get(),
            is_fpu_dirty: false,
        }
    }

    /// Restores the possibly modified registers when the tracee resumes.
    pub(crate) fn restore(self, user_context: &mut UserContext, supp_context: &SuppUserContext) {
        *user_context = self.user_context;
        supp_context.fs_base().set(self.fs_base);
        supp_context.gs_base().set(self.gs_base);
        if self.is_fpu_dirty {
            supp_context.fpu().set(self.fpu_context);
        }
    }

    /// Returns the saved syscall-return register value, including any tracer modifications.
    ///
    /// Returns `None` if the value indicates a non-syscall entry.
    pub(crate) fn orig_syscall_ret(&self) -> Option<usize> {
        (self.orig_syscall_ret != Self::NOT_A_SYSCALL).then_some(self.orig_syscall_ret)
    }

    /// Returns the x86-64 general-purpose register ABI snapshot.
    pub(crate) fn get_user_regs(&self) -> CUserRegsStruct {
        let mut regs = CUserRegsStruct::from_regs(&self.user_context, self.fs_base, self.gs_base);
        regs.orig_rax = self.orig_syscall_ret;
        regs
    }

    /// Applies an x86-64 general-purpose register ABI snapshot.
    pub(crate) fn set_user_regs(&mut self, regs: CUserRegsStruct) -> Result<()> {
        self.set_user_regs_prefix(regs, size_of::<CUserRegsStruct>())
    }

    fn set_user_regs_prefix(&mut self, regs: CUserRegsStruct, len: usize) -> Result<()> {
        regs.apply_prefix_to(
            &mut self.user_context,
            &mut self.fs_base,
            &mut self.gs_base,
            len,
        )?;
        if core::mem::offset_of!(CUserRegsStruct, orig_rax) < len {
            self.orig_syscall_ret = regs.orig_rax;
        }
        Ok(())
    }

    /// Copies a prefix of the register set into the writer.
    pub(crate) fn get_regset(&self, regset: Regset, writer: &mut VmWriter) -> Result<()> {
        match regset {
            Regset::General => {
                let regs = self.get_user_regs();
                writer.write_fallible(&mut VmReader::from(regs.as_bytes()))?;
            }
            Regset::Fp => {
                let regs = CUserFpregsStruct::from_context(&self.fpu_context);
                writer.write_fallible(&mut VmReader::from(regs.as_bytes()))?;
            }
        }
        Ok(())
    }

    /// Applies a register set from the reader.
    ///
    /// General-purpose registers permit prefix writes; FPU registers require
    /// the complete FXSAVE area.
    pub(crate) fn set_regset(&mut self, regset: Regset, reader: &mut VmReader) -> Result<()> {
        match regset {
            Regset::General => {
                if !reader.has_remain() {
                    return Ok(());
                }
                let mut regs = CUserRegsStruct::default();
                let len = reader.read_fallible(&mut VmWriter::from(regs.as_mut_bytes()))?;
                self.set_user_regs_prefix(regs, len)
            }
            Regset::Fp => {
                if reader.remain() != size_of::<CUserFpregsStruct>() {
                    return_errno_with_message!(Errno::EINVAL, "invalid FPU register set length");
                }
                let regs = reader.read_val::<CUserFpregsStruct>()?;
                regs.apply_to(&mut self.fpu_context)?;
                self.is_fpu_dirty = true;
                Ok(())
            }
        }
    }

    /// Reads one word from the x86-64 USER area.
    pub(crate) fn peek_user(&self, offset: usize) -> Result<usize> {
        check_user_offset(offset)?;
        if offset == core::mem::offset_of!(CUserRegsStruct, rflags) {
            return Ok(self.user_context.rflags());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, fsbase) {
            return Ok(self.fs_base.addr());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, gsbase) {
            return Ok(self.gs_base.addr());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, orig_rax) {
            return Ok(self.orig_syscall_ret);
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
            _ => (rule.get.unwrap())(self.user_context.general_regs()),
        })
    }

    /// Writes one word to the x86-64 USER area.
    pub(crate) fn poke_user(&mut self, offset: usize, value: usize) -> Result<()> {
        check_user_offset(offset)?;
        if offset == core::mem::offset_of!(CUserRegsStruct, rflags) {
            self.user_context.set_rflags(value);
            return Ok(());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, fsbase) {
            if !is_user_addr(value) {
                return_errno_with_message!(Errno::EIO, "invalid register value");
            }
            self.fs_base = FsBase::new(value);
            return Ok(());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, gsbase) {
            if !is_user_addr(value) {
                return_errno_with_message!(Errno::EIO, "invalid register value");
            }
            self.gs_base = GsBase::new(value);
            return Ok(());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, orig_rax) {
            self.orig_syscall_ret = value;
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
        rule.apply(self.user_context.general_regs_mut(), value)
    }

    /// Enables single-step execution.
    pub(crate) fn enable_single_step(&mut self) {
        self.user_context
            .set_rflags(self.user_context.rflags() | RFlags::TRAP_FLAG.bits() as usize);
    }

    /// Disables single-step execution.
    pub(crate) fn disable_single_step(&mut self) {
        self.user_context
            .set_rflags(self.user_context.rflags() & !(RFlags::TRAP_FLAG.bits() as usize));
    }
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
    assert!((REG_RULES.len() + 4) * size_of::<usize>() == size_of::<CUserRegsStruct>());
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
    /// The field is fixed at the expected value.
    Fixed(usize),
}

const fn is_user_addr(v: usize) -> bool {
    v < MAX_USERSPACE_VADDR
}

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
