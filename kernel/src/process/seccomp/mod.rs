// SPDX-License-Identifier: MPL-2.0

//! Secure computing (seccomp) mode support.
//!
//! Seccomp lets a thread irreversibly restrict the system calls it may make,
//! which is a building block for sandboxes and container runtimes. This module
//! implements the kernel-side pieces shared by [`seccomp(2)`], [`prctl(2)`], and
//! the system-call dispatch hook.
//!
//! # Modes
//!
//! - [`SeccompMode::Strict`] permits only `read`, `write`, `_exit`, and
//!   `rt_sigreturn`; any other call kills the thread with `SIGKILL`.
//! - [`SeccompMode::Filter`] evaluates a chain of classic-BPF programs against a
//!   [`SeccompData`] descriptor of the call and applies the action they return.
//!
//! # Filter chain
//!
//! Each `SECCOMP_SET_MODE_FILTER` prepends one [`SeccompFilter`] to an
//! append-only, immutable [`SeccompFilterChain`] held behind an `Arc`. A chain
//! is never mutated after creation, so evaluating it only clones the head `Arc`
//! and walks the links; sharing the `Arc` also lets a forked child inherit its
//! parent's filters for free. Following Linux, every filter in the chain runs
//! and the numerically smallest (most restrictive) action wins; ties keep the
//! data of the newest filter.
//!
//! # Hot path
//!
//! The dispatch hook must add no measurable cost to threads that never use
//! seccomp: such a thread pays a single relaxed atomic load of the mode, which
//! reports [`SeccompMode::Disabled`] and returns immediately. Only once a thread
//! is in filter mode does the hook take the per-thread chain lock long enough to
//! clone the head `Arc`, then build a [`SeccompData`] descriptor and evaluate.
//!
//! # Classic BPF
//!
//! The read-only cBPF subset that seccomp filters use lives in the [`bpf`]
//! submodule: [`run_filter`] interprets it and [`validate_filter`] rejects
//! malformed programs at install time so an accepted program can never reach an
//! undefined state at run time.
//!
//! The constants and ABI structures follow Linux `linux/seccomp.h`,
//! `linux/filter.h`, `linux/bpf_common.h`, and
//! `Documentation/userspace-api/seccomp_filter.rst`.
//!
//! [`seccomp(2)`]: https://man7.org/linux/man-pages/man2/seccomp.2.html
//! [`prctl(2)`]: https://man7.org/linux/man-pages/man2/prctl.2.html

mod bpf;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub use bpf::{BPF_MAXINSNS, run_filter, validate_filter};

use crate::prelude::*;

pub const SECCOMP_MODE_DISABLED: u32 = 0;
pub const SECCOMP_MODE_STRICT: u32 = 1;
pub const SECCOMP_MODE_FILTER: u32 = 2;

pub const SECCOMP_SET_MODE_STRICT: u32 = 0;
pub const SECCOMP_SET_MODE_FILTER: u32 = 1;
pub const SECCOMP_GET_ACTION_AVAIL: u32 = 2;

pub const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
pub const SECCOMP_RET_KILL_THREAD: u32 = 0x0000_0000;
pub const SECCOMP_RET_TRAP: u32 = 0x0003_0000;
pub const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
pub const SECCOMP_RET_TRACE: u32 = 0x7ff0_0000;
pub const SECCOMP_RET_LOG: u32 = 0x7ffc_0000;
pub const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
pub const SECCOMP_RET_USER_NOTIF: u32 = 0x7fc0_0000;
pub const SECCOMP_RET_ACTION_FULL: u32 = 0xffff_0000;
pub const SECCOMP_RET_DATA: u32 = 0x0000_ffff;

pub const SECCOMP_FILTER_FLAG_LOG: u32 = 1 << 1;
pub const SECCOMP_FILTER_FLAG_SPEC_ALLOW: u32 = 1 << 2;

/// The maximum cumulative instruction count across a thread's whole filter
/// chain, matching Linux's `MAX_INSNS_PER_PATH`.
const MAX_INSNS_PER_PATH: usize = 32768;
/// The per-installed-filter penalty added to the cumulative count, matching
/// Linux's accounting in `seccomp_attach_filter`.
const SECCOMP_FILTER_PENALTY: usize = 4;

#[cfg(target_arch = "x86_64")]
pub const AUDIT_ARCH_NATIVE: u32 = 0xc000_003e;
#[cfg(target_arch = "riscv64")]
pub const AUDIT_ARCH_NATIVE: u32 = 0xc000_00f3;
#[cfg(target_arch = "loongarch64")]
pub const AUDIT_ARCH_NATIVE: u32 = 0xc000_0102;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SeccompMode {
    Disabled = SECCOMP_MODE_DISABLED,
    Strict = SECCOMP_MODE_STRICT,
    Filter = SECCOMP_MODE_FILTER,
}

impl SeccompMode {
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct SeccompData {
    pub nr: i32,
    pub arch: u32,
    pub instruction_pointer: u64,
    pub args: [u64; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct SockFilter {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeccompAction {
    Allow,
    Log,
    Errno(u16),
    Trap(u16),
    Trace(u16),
    UserNotif(u16),
    KillThread,
    KillProcess,
}

impl SeccompAction {
    pub fn from_ret(ret: u32) -> Self {
        let data = (ret & SECCOMP_RET_DATA) as u16;
        match ret & SECCOMP_RET_ACTION_FULL {
            SECCOMP_RET_ALLOW => Self::Allow,
            SECCOMP_RET_LOG => Self::Log,
            SECCOMP_RET_ERRNO => Self::Errno(data),
            SECCOMP_RET_TRAP => Self::Trap(data),
            SECCOMP_RET_TRACE => Self::Trace(data),
            SECCOMP_RET_USER_NOTIF => Self::UserNotif(data),
            SECCOMP_RET_KILL_PROCESS => Self::KillProcess,
            SECCOMP_RET_KILL_THREAD => Self::KillThread,
            _ => Self::KillProcess,
        }
    }

    /// The chain selects the action with the smallest precedence value, so the
    /// ordering here encodes "most restrictive wins" from Linux's
    /// `seccomp_run_filters`.
    fn precedence(self) -> u8 {
        match self {
            Self::KillProcess => 0,
            Self::KillThread => 1,
            Self::Trap(_) => 2,
            Self::Errno(_) => 3,
            Self::UserNotif(_) => 4,
            Self::Trace(_) => 5,
            Self::Log => 6,
            Self::Allow => 7,
        }
    }
}

#[derive(Debug)]
pub struct SeccompFilter {
    program: Box<[SockFilter]>,
    log_non_allow: bool,
}

impl SeccompFilter {
    pub fn new(program: Box<[SockFilter]>, flags: u32) -> Result<Self> {
        validate_filter_flags(flags)?;
        validate_filter(&program)?;
        Ok(Self {
            program,
            log_non_allow: flags & SECCOMP_FILTER_FLAG_LOG != 0,
        })
    }

    pub fn evaluate(&self, data: &SeccompData) -> u32 {
        run_filter(&self.program, data)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeccompEvaluation {
    pub action: SeccompAction,
    pub should_log: bool,
}

#[derive(Debug)]
pub struct SeccompFilterChain {
    filter: SeccompFilter,
    previous: Option<Arc<SeccompFilterChain>>,
}

impl SeccompFilterChain {
    pub fn new(filter: SeccompFilter, previous: Option<Arc<Self>>) -> Arc<Self> {
        Arc::new(Self { filter, previous })
    }

    /// The number of filters in the chain, as reported by
    /// `/proc/[pid]/status`'s `Seccomp_filters` line.
    pub fn count(&self) -> usize {
        let mut count = 0;
        let mut current = Some(self);
        while let Some(chain) = current {
            count += 1;
            current = chain.previous.as_deref();
        }
        count
    }

    pub fn evaluate_with_metadata(&self, data: &SeccompData) -> SeccompEvaluation {
        let mut selected = SeccompAction::Allow;
        let mut should_log = false;
        let mut current = Some(self);

        while let Some(chain) = current {
            let action = SeccompAction::from_ret(chain.filter.evaluate(data));
            if chain.filter.log_non_allow && !matches!(action, SeccompAction::Allow) {
                should_log = true;
            }
            // Linux runs every filter and, for equal precedence, keeps the
            // newest filter's data. Since the chain is newest-first, do not
            // replace the selected action on ties.
            if action.precedence() < selected.precedence() {
                selected = action;
            }
            current = chain.previous.as_deref();
        }

        SeccompEvaluation {
            action: selected,
            should_log,
        }
    }
}

#[derive(Debug)]
pub struct SeccompState {
    mode: AtomicU32,
    no_new_privs: AtomicBool,
    filter_chain: RwLock<Option<Arc<SeccompFilterChain>>>,
}

impl SeccompState {
    pub fn new() -> Self {
        Self {
            mode: AtomicU32::new(SECCOMP_MODE_DISABLED),
            no_new_privs: AtomicBool::new(false),
            filter_chain: RwLock::new(None),
        }
    }

    pub fn fork_from(parent: &Self) -> Self {
        Self {
            mode: AtomicU32::new(parent.mode().as_u32()),
            no_new_privs: AtomicBool::new(parent.no_new_privs()),
            filter_chain: RwLock::new(parent.filter_chain()),
        }
    }

    pub fn mode(&self) -> SeccompMode {
        // Only `enable_strict`, `append_filter`, and `fork_from` ever write this
        // atomic, so it always holds a valid discriminant; an unexpected value
        // degrades safely to the most permissive mode rather than panicking on
        // this per-syscall hot path.
        match self.mode.load(Ordering::Relaxed) {
            SECCOMP_MODE_STRICT => SeccompMode::Strict,
            SECCOMP_MODE_FILTER => SeccompMode::Filter,
            _ => SeccompMode::Disabled,
        }
    }

    pub fn no_new_privs(&self) -> bool {
        self.no_new_privs.load(Ordering::Relaxed)
    }

    pub fn set_no_new_privs(&self) {
        self.no_new_privs.store(true, Ordering::Relaxed);
    }

    pub fn filter_chain(&self) -> Option<Arc<SeccompFilterChain>> {
        self.filter_chain.read().clone()
    }

    /// The number of installed filters, for `/proc/[pid]/status`.
    pub fn filter_count(&self) -> usize {
        self.filter_chain
            .read()
            .as_ref()
            .map_or(0, |chain| chain.count())
    }

    pub fn enable_strict(&self) -> Result<()> {
        self.mode
            .compare_exchange(
                SECCOMP_MODE_DISABLED,
                SECCOMP_MODE_STRICT,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .map(|_| ())
            .map_err(|_| Error::with_message(Errno::EINVAL, "seccomp mode is already enabled"))
    }

    pub fn append_filter(&self, filter: SeccompFilter) -> Result<()> {
        // Hold the chain lock across the mode transition so the chain is never
        // observed empty while the mode reads `Filter`. The transition mirrors
        // `enable_strict`: `Disabled` becomes `Filter`, an existing `Filter`
        // accepts another filter, and `Strict` is never downgraded to `Filter`.
        let mut chain = self.filter_chain.write();

        // Bound the cumulative program length across the whole chain, like
        // Linux's `MAX_INSNS_PER_PATH`, so an unprivileged thread cannot stack
        // filters to make every system call arbitrarily expensive. Each
        // already-installed filter also costs a small fixed penalty, matching
        // the kernel.
        let mut total = filter.program.len();
        let mut node = chain.as_deref();
        while let Some(current) = node {
            total += current.filter.program.len() + SECCOMP_FILTER_PENALTY;
            node = current.previous.as_deref();
        }
        if total > MAX_INSNS_PER_PATH {
            return_errno_with_message!(Errno::ENOMEM, "seccomp filter chain is too large");
        }

        match self.mode.compare_exchange(
            SECCOMP_MODE_DISABLED,
            SECCOMP_MODE_FILTER,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) | Err(SECCOMP_MODE_FILTER) => {}
            Err(_) => return_errno_with_message!(
                Errno::EINVAL,
                "cannot add seccomp filters in strict mode"
            ),
        }
        let new_chain = SeccompFilterChain::new(filter, chain.clone());
        *chain = Some(new_chain);
        Ok(())
    }
}

impl Default for SeccompState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn is_action_available(action: u32) -> bool {
    if action & !SECCOMP_RET_ACTION_FULL != 0 {
        return false;
    }

    matches!(
        action,
        SECCOMP_RET_KILL_PROCESS
            | SECCOMP_RET_KILL_THREAD
            | SECCOMP_RET_TRAP
            | SECCOMP_RET_ERRNO
            | SECCOMP_RET_TRACE
            | SECCOMP_RET_LOG
            | SECCOMP_RET_ALLOW
            | SECCOMP_RET_USER_NOTIF
    )
}

pub fn validate_filter_flags(flags: u32) -> Result<()> {
    let supported = SECCOMP_FILTER_FLAG_LOG | SECCOMP_FILTER_FLAG_SPEC_ALLOW;
    if flags & !supported != 0 {
        return_errno_with_message!(Errno::EINVAL, "unsupported seccomp filter flags");
    }
    Ok(())
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    fn sample_data(nr: i32) -> SeccompData {
        SeccompData {
            nr,
            arch: AUDIT_ARCH_NATIVE,
            instruction_pointer: 0x1234_5678_9abc_def0,
            args: [1, 2, 3, 4, 5, 6],
        }
    }

    fn filter(ret: u32) -> SeccompFilter {
        SeccompFilter::new(bpf::ret_program(ret), 0).unwrap()
    }

    #[ktest]
    fn seccomp() {
        seccomp_chain_keeps_most_restrictive_action();
        seccomp_chain_keeps_newest_data_for_equal_precedence();
        seccomp_state_transitions_are_one_way();
        seccomp_fork_inherits_state_and_shares_chain();
        seccomp_state_bounds_total_filter_instructions();
    }

    #[ktest]
    fn seccomp_chain_keeps_most_restrictive_action() {
        let chain = SeccompFilterChain::new(filter(SECCOMP_RET_ALLOW), None);
        let chain = SeccompFilterChain::new(
            filter(SECCOMP_RET_ERRNO | Errno::EACCES as u32),
            Some(chain),
        );
        let chain = SeccompFilterChain::new(filter(SECCOMP_RET_KILL_PROCESS), Some(chain));

        assert_eq!(
            chain.evaluate_with_metadata(&sample_data(0)).action,
            SeccompAction::KillProcess
        );
    }

    #[ktest]
    fn seccomp_chain_keeps_newest_data_for_equal_precedence() {
        let chain = SeccompFilterChain::new(filter(SECCOMP_RET_ERRNO | Errno::EPERM as u32), None);
        let chain = SeccompFilterChain::new(
            filter(SECCOMP_RET_ERRNO | Errno::EACCES as u32),
            Some(chain),
        );

        assert_eq!(
            chain.evaluate_with_metadata(&sample_data(0)).action,
            SeccompAction::Errno(Errno::EACCES as u16)
        );
    }

    #[ktest]
    fn seccomp_state_transitions_are_one_way() {
        // `Disabled` enters `Filter` and then accepts further filters.
        let state = SeccompState::new();
        assert_eq!(state.mode(), SeccompMode::Disabled);
        assert!(state.filter_chain().is_none());
        state.append_filter(filter(SECCOMP_RET_ALLOW)).unwrap();
        assert_eq!(state.mode(), SeccompMode::Filter);
        assert!(state.filter_chain().is_some());
        state.append_filter(filter(SECCOMP_RET_ALLOW)).unwrap();
        assert_eq!(state.mode(), SeccompMode::Filter);

        // `Strict` is one-way and rejects both a second enable and any filter.
        let state = SeccompState::new();
        state.enable_strict().unwrap();
        assert_eq!(state.mode(), SeccompMode::Strict);
        assert_eq!(state.enable_strict().unwrap_err().error(), Errno::EINVAL);
        assert_eq!(
            state
                .append_filter(filter(SECCOMP_RET_ALLOW))
                .unwrap_err()
                .error(),
            Errno::EINVAL
        );
        assert_eq!(state.mode(), SeccompMode::Strict);
    }

    #[ktest]
    fn seccomp_fork_inherits_state_and_shares_chain() {
        let parent = SeccompState::new();
        parent.set_no_new_privs();
        parent.append_filter(filter(SECCOMP_RET_ALLOW)).unwrap();

        let child = SeccompState::fork_from(&parent);
        assert!(child.no_new_privs());
        assert_eq!(child.mode(), SeccompMode::Filter);
        assert!(Arc::ptr_eq(
            &parent.filter_chain().unwrap(),
            &child.filter_chain().unwrap()
        ));
    }

    #[ktest]
    fn seccomp_state_bounds_total_filter_instructions() {
        let state = SeccompState::new();
        let big =
            || SeccompFilter::new(bpf::padded_program(BPF_MAXINSNS, SECCOMP_RET_ALLOW), 0).unwrap();

        // Each maximum-size filter consumes most of the cumulative budget, so
        // after a few installs the next one must be rejected with `ENOMEM`.
        let mut installed = 0;
        let mut hit_limit = false;
        for _ in 0..16 {
            match state.append_filter(big()) {
                Ok(()) => installed += 1,
                Err(err) => {
                    assert_eq!(err.error(), Errno::ENOMEM);
                    hit_limit = true;
                    break;
                }
            }
        }
        assert!(installed >= 1);
        assert!(hit_limit);
    }
}
