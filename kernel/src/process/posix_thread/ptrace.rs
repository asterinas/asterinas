// SPDX-License-Identifier: MPL-2.0

#[cfg(target_arch = "x86_64")]
use core::mem::{offset_of, size_of};
use core::sync::atomic::{AtomicBool, Ordering};

use hashbrown::HashMap;
#[cfg(target_arch = "x86_64")]
use ostd::{
    arch::cpu::context::{GeneralRegs, UserContext, c_user_regs_struct},
    user::UserContextApi,
};

use super::{AsPosixThread, PosixThread};
use crate::{
    prelude::*,
    process::signal::{c_types::siginfo_t, sig_num::SigNum, signals::Signal},
    thread::{Thread, Tid},
};

impl PosixThread {
    /// Returns whether this thread may be a tracee.
    pub(in crate::process) fn may_be_tracee(&self) -> bool {
        self.tracee_status.is_completed()
    }

    /// Returns the tracer of this thread if it is being traced.
    pub fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracee_status.get().and_then(|status| status.tracer())
    }

    /// Sets the tracer of this thread.
    ///
    /// # Errors
    ///
    /// Returns `EPERM` if the this thread is already being traced.
    pub fn set_tracer(&self, tracer: Weak<Thread>) -> Result<()> {
        let status = self.tracee_status.call_once(TraceeStatus::new);
        status.set_tracer(tracer)
    }

    /// Detaches the tracer of this thread.
    pub fn detach_tracer(&self) {
        if let Some(status) = self.tracee_status.get() {
            status.detach_tracer(
                // Lock order: user_ctx -> ptrace_status.
                #[cfg(target_arch = "x86_64")]
                &mut self.user_ctx().lock(),
            );
            self.wake_signalled_waker();
        }
    }

    /// Stops this thread by ptrace with the given signal if it is currently traced.
    ///
    /// Returns:
    /// - Ok(()) if the thread is traced and the ptrace-stop is triggered.
    /// - Err(signal) if the thread is not being traced, returning the original signal.
    pub(in crate::process) fn ptrace_stop(
        &self,
        signal: Box<dyn Signal>,
    ) -> core::result::Result<(), Box<dyn Signal>> {
        if let Some(status) = self.tracee_status.get() {
            status.ptrace_stop(signal)
        } else {
            Err(signal)
        }
    }

    /// Returns whether this thread is stopped by ptrace.
    pub fn is_ptrace_stopped(&self) -> bool {
        if let Some(status) = self.tracee_status.get() {
            status.is_ptrace_stopped()
        } else {
            false
        }
    }

    /// Gets and clears the ptrace-stop status changes for the `wait` syscall.
    pub(in crate::process) fn wait_ptrace_stopped(&self) -> Option<SigNum> {
        self.tracee_status.get().and_then(|status| status.wait())
    }

    /// Continues this thread from a ptrace-stop.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    pub fn ptrace_continue(&self, request: PtraceContRequest) -> Result<()> {
        let Some(status) = self.tracee_status.get() else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not being traced");
        };

        status.resume(
            request,
            // Lock order: user_ctx -> ptrace_status.
            #[cfg(target_arch = "x86_64")]
            &mut self.user_ctx().lock(),
        )?;
        self.wake_signalled_waker();

        Ok(())
    }

    /// Gets the general-purpose registers of this thread for ptrace.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_get_regs(&self) -> Result<GeneralRegs> {
        let Some(status) = self.tracee_status.get() else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not being traced");
        };

        // Lock order: user_ctx -> ptrace_status.
        status.get_regs(&self.user_ctx().lock())
    }

    /// Sets the general-purpose registers of this thread for ptrace.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_set_regs(&self, regs: c_user_regs_struct) -> Result<()> {
        let Some(status) = self.tracee_status.get() else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not being traced");
        };

        // Lock order: user_ctx -> ptrace_status.
        status.set_regs(&mut self.user_ctx().lock(), regs)
    }

    /// Reads one word in the tracee's USER area.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_peek_user(&self, offset: usize) -> Result<usize> {
        let Some(status) = self.tracee_status.get() else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not being traced");
        };

        // Lock order: user_ctx -> ptrace_status.
        status.peek_user(&self.user_ctx().lock(), offset)
    }

    /// Writes one word in the tracee's USER area.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_poke_user(&self, offset: usize, value: usize) -> Result<()> {
        let Some(status) = self.tracee_status.get() else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not being traced");
        };

        // Lock order: user_ctx -> ptrace_status.
        status.poke_user(&mut self.user_ctx().lock(), offset, value)
    }

    /// Gets the waited signal info of this thread for ptrace.
    pub fn ptrace_get_siginfo(&self) -> Result<siginfo_t> {
        let Some(status) = self.tracee_status.get() else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not being traced");
        };
        status.get_siginfo()
    }
}

impl PosixThread {
    /// Inserts a tracee to the tracee map of this thread.
    ///
    /// # Panics
    ///
    /// This method will panic if the tracee is not a POSIX thread.
    pub fn insert_tracee(&self, tracee: Arc<Thread>) {
        let tracees = self.tracees.call_once(|| Mutex::new(HashMap::new()));
        tracees
            .lock()
            .insert(tracee.as_posix_thread().unwrap().tid(), tracee);
    }

    /// Removes the tracee with the given tid from the tracee map of this thread.
    #[expect(dead_code)]
    pub fn remove_tracee(&self, tid: Tid) {
        if let Some(tracees) = self.tracees.get() {
            tracees.lock().remove(&tid);
        }
    }

    /// Returns the tracee map of this thread if it is a tracer.
    pub(in crate::process) fn tracees(&self) -> Option<&Mutex<HashMap<Tid, Arc<Thread>>>> {
        self.tracees.get()
    }

    /// Returns the tracee with the given tid, if it is being traced by this thread.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if there is no tracee with the given tid.
    pub fn get_tracee(&self, tid: Tid) -> Result<Arc<Thread>> {
        self.tracees()
            .and_then(|tracees| tracees.lock().get(&tid).cloned())
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "no such tracee"))
    }
}

pub(super) struct TraceeStatus {
    is_stopped: AtomicBool,
    state: Mutex<TraceeState>,
}

impl TraceeStatus {
    pub(super) fn new() -> Self {
        Self {
            is_stopped: AtomicBool::new(false),
            state: Mutex::new(TraceeState::new()),
        }
    }

    fn tracer(&self) -> Option<Arc<Thread>> {
        self.state.lock().tracer()
    }

    fn set_tracer(&self, tracer: Weak<Thread>) -> Result<()> {
        let mut state = self.state.lock();
        if state.tracer().is_some() {
            return_errno_with_message!(Errno::EPERM, "the thread is already being traced");
        }
        state.set_tracer(tracer);

        Ok(())
    }

    fn detach_tracer(
        &self,
        #[cfg(target_arch = "x86_64")] user_ctx: &mut MutexGuard<'_, UserContext>,
    ) {
        // Hold the lock first to avoid race conditions.
        let mut tracee_state = self.state.lock();

        tracee_state.detach_tracer();
        #[cfg(target_arch = "x86_64")]
        {
            user_ctx.set_single_step(false);
        }
        tracee_state.siginfo = None;
        self.is_stopped.store(false, Ordering::Relaxed);
    }

    fn ptrace_stop(&self, signal: Box<dyn Signal>) -> core::result::Result<(), Box<dyn Signal>> {
        // Hold the lock first to avoid race conditions.
        let mut tracee_state = self.state.lock();

        let Some(tracer) = tracee_state.tracer() else {
            return Err(signal);
        };

        if !self.is_stopped.load(Ordering::Relaxed) {
            self.is_stopped.store(true, Ordering::Relaxed);
            tracee_state.siginfo = Some(signal.to_info());
            let tracer_process = tracer.as_posix_thread().unwrap().process();
            tracer_process.children_wait_queue().wake_all();
        }

        Ok(())
    }

    fn is_ptrace_stopped(&self) -> bool {
        self.is_stopped.load(Ordering::Relaxed)
    }

    fn wait(&self) -> Option<SigNum> {
        // Hold the lock first to avoid race conditions.
        let mut tracee_state = self.state.lock();

        if let Some(siginfo) = tracee_state.siginfo.take() {
            let sig_num = (siginfo.si_signo as u8).try_into().unwrap();
            tracee_state.waited_siginfo = Some(siginfo);
            Some(sig_num)
        } else {
            None
        }
    }

    fn resume(
        &self,
        request: PtraceContRequest,
        #[cfg(target_arch = "x86_64")] user_ctx: &mut MutexGuard<'_, UserContext>,
    ) -> Result<()> {
        debug!("resuming from ptrace-stop with request: {:?}", request);

        // Hold the lock first to avoid race conditions.
        let mut tracee_state = self.state.lock();

        if self.is_stopped.load(Ordering::Relaxed) {
            #[cfg(target_arch = "x86_64")]
            {
                user_ctx.set_single_step(matches!(request, PtraceContRequest::SingleStep));
            }
            tracee_state.siginfo = None;
            self.is_stopped.store(false, Ordering::Relaxed);
        } else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not ptrace-stopped");
        }

        Ok(())
    }

    #[cfg(target_arch = "x86_64")]
    fn get_regs(&self, user_ctx: &MutexGuard<'_, UserContext>) -> Result<GeneralRegs> {
        // Hold the lock first to avoid race conditions.
        let _tracee_state = self.state.lock();

        if self.is_stopped.load(Ordering::Relaxed) {
            Ok(*user_ctx.general_regs())
        } else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not ptrace-stopped");
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn set_regs(
        &self,
        user_ctx: &mut MutexGuard<'_, UserContext>,
        regs: c_user_regs_struct,
    ) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let _tracee_state = self.state.lock();
        if !self.is_stopped.load(Ordering::Relaxed) {
            return_errno_with_message!(Errno::ESRCH, "the thread is not ptrace-stopped");
        }

        macro_rules! set_regs_from_user_regs {
            ($old_regs:ident, $regs:ident, [ $field:ident, $($meta:tt)+ ]) => {
                paste::paste! {
                    [<ptrace_set_ $field>](&mut $old_regs, $regs.$field)?;
                }
            };
        }
        let mut old_regs = *user_ctx.general_regs();
        ostd::for_all_general_regs!(set_regs_from_user_regs, old_regs, regs);

        *user_ctx.general_regs_mut() = old_regs;
        Ok(())
    }

    #[cfg(target_arch = "x86_64")]
    fn peek_user(&self, user_ctx: &MutexGuard<'_, UserContext>, offset: usize) -> Result<usize> {
        // Hold the lock first to avoid race conditions.
        let _tracee_state = self.state.lock();
        if !self.is_stopped.load(Ordering::Relaxed) {
            return_errno_with_message!(Errno::ESRCH, "the thread is not ptrace-stopped");
        }
        check_user_offset(offset)?;

        macro_rules! read_user_reg_by_offset {
            ($regs:ident, $offset:ident, [ $field:ident, $($meta:tt)+ ]) => {
                if $offset == offset_of!(c_user_regs_struct, $field) {
                    return Ok($regs.$field());
                }
            };
        }
        let regs = user_ctx.general_regs();
        ostd::for_all_general_regs!(read_user_reg_by_offset, regs, offset);

        unreachable!("the offset is valid in `c_user_regs_struct`")
    }

    #[cfg(target_arch = "x86_64")]
    fn poke_user(
        &self,
        user_ctx: &mut MutexGuard<'_, UserContext>,
        offset: usize,
        value: usize,
    ) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let _tracee_state = self.state.lock();
        if !self.is_stopped.load(Ordering::Relaxed) {
            return_errno_with_message!(Errno::ESRCH, "the thread is not ptrace-stopped");
        }
        check_user_offset(offset)?;

        macro_rules! write_user_reg_by_offset {
            ($regs:ident, $offset:ident, $value:ident, [ $field:ident, $($meta:tt)+ ]) => {
                if $offset == offset_of!(c_user_regs_struct, $field) {
                    paste::paste! {
                        [<ptrace_set_ $field>](&mut $regs, $value)?;
                        return Ok(());
                    }
                }
            };
        }
        let mut regs = user_ctx.general_regs_mut();
        ostd::for_all_general_regs!(write_user_reg_by_offset, regs, offset, value);

        unreachable!("the offset is valid in `c_user_regs_struct`")
    }

    fn get_siginfo(&self) -> Result<siginfo_t> {
        // Hold the lock first to avoid race conditions.
        let tracee_state = self.state.lock();

        if self.is_stopped.load(Ordering::Relaxed)
            && let Some(siginfo) = tracee_state.waited_siginfo
        {
            Ok(siginfo)
        } else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not ptrace-stopped");
        }
    }
}

struct TraceeState {
    tracer: Weak<Thread>,
    /// The siginfo of the signal that stopped the tracee and has not yet been waited on.
    siginfo: Option<siginfo_t>,
    /// The siginfo of the signal that stopped the tracee and has already been waited on.
    ///
    /// This is needed to support `PTRACE_GETSIGINFO`.
    waited_siginfo: Option<siginfo_t>,
}

impl TraceeState {
    fn new() -> Self {
        Self {
            tracer: Weak::new(),
            siginfo: None,
            waited_siginfo: None,
        }
    }

    fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracer.upgrade()
    }

    fn set_tracer(&mut self, tracer: Weak<Thread>) {
        self.tracer = tracer;
    }

    fn detach_tracer(&mut self) {
        self.tracer = Weak::new();
    }
}

/// The requests that can continue a stopped tracee.
#[expect(dead_code)]
#[derive(Debug)]
pub enum PtraceContRequest {
    Continue,
    SingleStep,
    Syscall,
}

#[cfg(target_arch = "x86_64")]
macro_rules! general_regs_ptrace_setter {
    ([ $field:ident, $($meta:tt)+ ]) => {
        paste::paste! {
            #[inline(always)]
            fn [<ptrace_set_ $field>](regs: &mut GeneralRegs, value: usize) -> Result<()> {
                general_regs_ptrace_setter!(@body regs, value, [ $field, $($meta)+ ]);
                Ok(())
            }
        }
    };

    (@body $regs:ident, $value:ident, [ $field:ident, set ]) => {{
        paste::paste! {
            $regs.[<set_ $field>]($value);
        }
    }};

    (@body $regs:ident, $value:ident, [ $field:ident, set_if($check:expr) ]) => {{
        if ($check)($value) {
            paste::paste! {
                $regs.[<set_ $field>]($value);
            }
        } else {
            return Err(Error::with_message(Errno::EIO, "invalid register value"));
        }
    }};

    (@body $regs:ident, $value:ident, [ $field:ident, set_bits_truncate($mask:expr) ]) => {{
        let old_value = $regs.$field();
        const MASK: usize = $mask;
        paste::paste! {
            $regs.[<set_ $field>]((old_value & !MASK) | ($value & MASK));
        }
    }};

    (@body $regs:ident, $value:ident, [ $field:ident, fixed($expected:expr) ]) => {{
        let _ = $regs;
        const EXPECTED: usize = $expected;
        if $value != EXPECTED {
            return Err(Error::with_message(Errno::EIO, "invalid segment selector"));
        }
    }};
}

#[cfg(target_arch = "x86_64")]
ostd::for_all_general_regs!(general_regs_ptrace_setter);

/// Checks whether the given offset is valid for in `struct user`.
//
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/include/asm/user_64.h#L103-L132>
#[cfg(target_arch = "x86_64")]
fn check_user_offset(offset: usize) -> Result<()> {
    if !offset.is_multiple_of(size_of::<usize>()) {
        return_errno_with_message!(Errno::EIO, "invalid USER area offset");
    }

    // We only support the offsets for general-purpose registers currently.
    // `struct user_regs_struct` is the first field in `struct user`.
    if offset >= size_of::<c_user_regs_struct>() {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "only offsets for general-purpose registers are supported currently"
        );
    }
    Ok(())
}
