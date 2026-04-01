// SPDX-License-Identifier: MPL-2.0

#[cfg(target_arch = "x86_64")]
use core::mem::{offset_of, size_of};
use core::sync::atomic::{AtomicBool, Ordering};

use hashbrown::HashMap;
#[cfg(target_arch = "x86_64")]
use ostd::arch::cpu::context::{GeneralRegs, c_user_regs_struct};
use ostd::{arch::cpu::context::UserContext, sync::Waiter};

use super::{AsPosixThread, PosixThread};
use crate::{
    prelude::*,
    process::{
        CloneArgs, CloneFlags,
        signal::{
            PauseReason,
            c_types::siginfo_t,
            constants::{SIGCHLD, SIGKILL, SIGTRAP},
            signals::{Signal, user::UserSignal},
        },
    },
    thread::{Thread, Tid},
};

mod util;

use util::StopSigInfo;
pub use util::{PtraceContRequest, PtraceOptions, PtraceWaitStatus};
pub(in crate::process) use util::{PtraceEvent, PtraceStopResult};

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
            status.detach_tracer();
            self.wake_signalled_waker();
        }
    }

    /// Stops this thread by ptrace with the given signal if it is currently traced.
    ///
    /// Returns:
    /// - `PtraceStopResult::Continued` if the ptrace-stop is continued by the tracer.
    /// - `PtraceStopResult::Interrupted` if the ptrace-stop is interrupted by `SIGKILL`.
    /// - `PtraceStopResult::NotTraced(signal)` if the thread is not traced,
    ///   with the given `signal` returned back.
    pub(in crate::process) fn ptrace_stop(
        &self,
        signal: Box<dyn Signal>,
        ctx: &Context,
        user_ctx: &mut UserContext,
    ) -> PtraceStopResult {
        if let Some(status) = self.tracee_status.get() {
            status.ptrace_stop(signal, ctx, user_ctx)
        } else {
            PtraceStopResult::NotTraced(signal)
        }
    }

    /// Stops this thread by ptrace on the given event if it is currently traced,
    /// and the corresponding option is enabled.
    pub(in crate::process) fn ptrace_may_stop_on(
        &self,
        event: PtraceEvent,
        ctx: &Context,
        user_ctx: &mut UserContext,
    ) {
        if let Some(status) = self.tracee_status.get() {
            status.ptrace_may_stop_on(event, ctx, user_ctx)
        }
    }

    /// Returns whether a clone-family ptrace event would be required for `clone_args`.
    pub(in crate::process) fn needs_ptrace_clone_stop(&self, clone_args: &CloneArgs) -> bool {
        self.tracee_status
            .get()
            .is_some_and(|status| status.needs_clone_stop(clone_args))
    }

    /// Gets and clears the ptrace-stop status changes for the `wait` syscall.
    pub(in crate::process) fn wait_ptrace_stopped(&self) -> Option<PtraceWaitStatus> {
        self.tracee_status.get().and_then(|status| status.wait())
    }

    /// Continues this thread from a ptrace-stop.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    pub fn ptrace_continue(&self, request: PtraceContRequest) -> Result<()> {
        let status = self.get_tracee_status()?;

        status.resume(request)?;
        self.wake_signalled_waker();

        Ok(())
    }

    /// Gets the general-purpose registers of this thread for ptrace.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_get_regs(&self) -> Result<c_user_regs_struct> {
        let status = self.get_tracee_status()?;
        status.get_regs()
    }

    /// Sets the general-purpose registers of this thread for ptrace.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_set_regs(&self, regs: c_user_regs_struct) -> Result<()> {
        let status = self.get_tracee_status()?;
        status.set_regs(regs)
    }

    /// Reads one word in the tracee's USER area.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_peek_user(&self, offset: usize) -> Result<usize> {
        let status = self.get_tracee_status()?;
        status.peek_user(offset)
    }

    /// Writes one word in the tracee's USER area.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_poke_user(&self, offset: usize, value: usize) -> Result<()> {
        let status = self.get_tracee_status()?;
        status.poke_user(offset, value)
    }

    /// Sets ptrace options for this thread.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    pub fn ptrace_set_options(&self, options: PtraceOptions) -> Result<()> {
        let status = self.get_tracee_status()?;
        status.set_options(options)
    }

    /// Gets the extra message of the last ptrace event stop.
    ///
    /// Returns 0 if the last ptrace-stop is not a ptrace-event-stop.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    pub fn ptrace_get_eventmsg(&self) -> Result<usize> {
        let status = self.get_tracee_status()?;
        status.get_eventmsg()
    }

    /// Gets the waited signal info of this thread for ptrace.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    pub fn ptrace_get_siginfo(&self) -> Result<siginfo_t> {
        let status = self.get_tracee_status()?;
        status.get_siginfo()
    }

    /// Returns the tracee status of this thread if has ever been traced.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread has never been traced.
    fn get_tracee_status(&self) -> Result<&TraceeStatus> {
        self.tracee_status
            .get()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the thread has never been traced"))
    }

    /// Returns the locked tracee state of this thread has ever been traced.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread has never been traced.
    fn get_state_locked(&self) -> Result<MutexGuard<'_, TraceeState>> {
        let status = self.get_tracee_status()?;
        Ok(status.state.lock())
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

    /// Clears all tracees of this tracer on exit.
    pub(in crate::process) fn clear_tracees(&self, ctx: &Context) {
        let Some(tracees) = self.tracees() else {
            return;
        };

        // Lock order: tracer.tracees -> tracee.tracee_status
        let tracees = tracees.lock();
        for (_, tracee) in tracees.iter() {
            let tracee = tracee.as_posix_thread().unwrap();
            tracee.detach_tracer();

            let tracee_state = tracee.get_state_locked().unwrap();
            if tracee_state
                .options
                .contains(PtraceOptions::PTRACE_O_EXITKILL)
            {
                tracee.enqueue_signal(Box::new(UserSignal::new_kill(SIGKILL, ctx)));
            }
        }
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
        state.tracer = tracer;

        Ok(())
    }

    fn detach_tracer(&self) {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();

        state.tracer = Weak::new();
        state.siginfo.clear();
        state.event = None;
        #[cfg(target_arch = "x86_64")]
        {
            if let Some(regs) = state.general_regs.as_mut() {
                regs.set_single_step(false);
            }
        }
        self.is_stopped.store(false, Ordering::Relaxed);
    }

    fn ptrace_stop(
        &self,
        signal: Box<dyn Signal>,
        ctx: &Context,
        user_ctx: &mut UserContext,
    ) -> PtraceStopResult {
        #[cfg(not(target_arch = "x86_64"))]
        let _ = user_ctx;

        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();

        if state.tracer().is_none() {
            return PtraceStopResult::NotTraced(signal);
        }

        self.do_ptrace_stop(state, signal.to_info(), None, ctx, user_ctx)
    }

    fn ptrace_may_stop_on(&self, event: PtraceEvent, ctx: &Context, user_ctx: &mut UserContext) {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();

        if state.tracer().is_none() {
            return;
        }

        if !state.options.contains(event.option()) {
            // If the PTRACE_O_TRACEEXEC option is not in effect, all successful
            // calls to execve(2) by the traced process will cause it to be sent
            // a SIGTRAP signal, giving the parent a chance to gain control
            // before the new program begins execution.
            //
            // Reference: <https://man7.org/linux/man-pages/man2/ptrace.2.html>
            if matches!(&event, PtraceEvent::Exec(_)) {
                ctx.posix_thread
                    .enqueue_signal(Box::new(UserSignal::new_kill(SIGTRAP, ctx)));
            }
            return;
        }

        let siginfo = event.siginfo(ctx);

        self.do_ptrace_stop(state, siginfo, Some(event), ctx, user_ctx);
    }

    fn do_ptrace_stop(
        &self,
        mut state: MutexGuard<'_, TraceeState>,
        siginfo: siginfo_t,
        event: Option<PtraceEvent>,
        ctx: &Context,
        user_ctx: &mut UserContext,
    ) -> PtraceStopResult {
        #[cfg(not(target_arch = "x86_64"))]
        let _ = user_ctx;

        debug_assert!(!self.is_ptrace_stopped());

        state.siginfo.stop(siginfo);
        state.event = event;
        #[cfg(target_arch = "x86_64")]
        {
            state.general_regs = Some(*user_ctx.general_regs());
        }
        self.is_stopped.store(true, Ordering::Relaxed);

        let tracer = state.tracer().unwrap();
        let tracer = tracer.as_posix_thread().unwrap();
        tracer.enqueue_signal(Box::new(UserSignal::new_kill(SIGCHLD, ctx)));
        tracer.process().children_wait_queue().wake_all();

        drop(state);

        let waiter = Waiter::new_pair().0;
        if waiter
            .pause_until_by(
                || (!self.is_ptrace_stopped()).then_some(()),
                PauseReason::StopByPtrace,
            )
            .is_err()
        {
            // A `SIGKILL` interrupts this ptrace-stop.
            return PtraceStopResult::Interrupted;
        };

        #[cfg(target_arch = "x86_64")]
        {
            let mut state = self.state.lock();
            let regs = state.general_regs.take().unwrap();
            *user_ctx.general_regs_mut() = regs;
        }

        PtraceStopResult::Continued
    }

    fn needs_clone_stop(&self, clone_args: &CloneArgs) -> bool {
        let state = self.state.lock();
        if state.tracer().is_none() {
            return false;
        }
        let options = state.options;

        if clone_args.flags.contains(CloneFlags::CLONE_VFORK) {
            return options.contains(PtraceOptions::PTRACE_O_TRACEVFORK)
                || options.contains(PtraceOptions::PTRACE_O_TRACEVFORKDONE);
        }

        if clone_args.exit_signal == Some(SIGCHLD) {
            return options.contains(PtraceOptions::PTRACE_O_TRACEFORK);
        }

        options.contains(PtraceOptions::PTRACE_O_TRACECLONE)
    }

    fn is_ptrace_stopped(&self) -> bool {
        self.is_stopped.load(Ordering::Relaxed)
    }

    fn check_ptrace_stopped(&self, _state_guard: &MutexGuard<'_, TraceeState>) -> Result<()> {
        if self.is_ptrace_stopped() {
            Ok(())
        } else {
            return_errno_with_message!(Errno::ESRCH, "the thread is not ptrace-stopped");
        }
    }

    fn wait(&self) -> Option<PtraceWaitStatus> {
        let mut state = self.state.lock();
        state.siginfo.wait().map(|siginfo| siginfo.into())
    }

    fn resume(&self, request: PtraceContRequest) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();
        self.check_ptrace_stopped(&state)?;
        debug!("resuming from ptrace-stop by request: {:?}", request);

        state.siginfo.clear();
        state.event = None;
        #[cfg(target_arch = "x86_64")]
        {
            let regs = state.general_regs.as_mut().unwrap();
            regs.set_single_step(matches!(request, PtraceContRequest::SingleStep));
        }
        self.is_stopped.store(false, Ordering::Relaxed);

        Ok(())
    }

    #[cfg(target_arch = "x86_64")]
    fn get_regs(&self) -> Result<c_user_regs_struct> {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        Ok(state.general_regs.unwrap().into())
    }

    #[cfg(target_arch = "x86_64")]
    fn set_regs(&self, regs: c_user_regs_struct) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        macro_rules! set_regs_from_user_regs {
            ($old_regs:ident, $regs:ident, [ $field:ident, $($meta:tt)+ ]) => {
                paste::paste! {
                    util::[<ptrace_set_ $field>](&mut $old_regs, $regs.$field)?;
                }
            };
        }
        let mut old_regs = state.general_regs.unwrap();
        ostd::for_all_general_regs!(set_regs_from_user_regs, old_regs, regs);

        state.general_regs = Some(old_regs);
        Ok(())
    }

    #[cfg(target_arch = "x86_64")]
    fn peek_user(&self, offset: usize) -> Result<usize> {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();
        self.check_ptrace_stopped(&state)?;
        util::check_user_offset(offset)?;

        macro_rules! read_user_reg_by_offset {
            ($regs:ident, $offset:ident, [ $field:ident, $($meta:tt)+ ]) => {
                if $offset == offset_of!(c_user_regs_struct, $field) {
                    return Ok($regs.$field());
                }
            };
        }
        let regs = state.general_regs.unwrap();
        ostd::for_all_general_regs!(read_user_reg_by_offset, regs, offset);

        unreachable!("the offset is valid in `c_user_regs_struct`")
    }

    #[cfg(target_arch = "x86_64")]
    fn poke_user(&self, offset: usize, value: usize) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();
        self.check_ptrace_stopped(&state)?;
        util::check_user_offset(offset)?;

        macro_rules! write_user_reg_by_offset {
            ($regs:ident, $offset:ident, $value:ident, [ $field:ident, $($meta:tt)+ ]) => {
                if $offset == offset_of!(c_user_regs_struct, $field) {
                    paste::paste! {
                        util::[<ptrace_set_ $field>](&mut $regs, $value)?;
                        return Ok(());
                    }
                }
            };
        }
        let mut regs = state.general_regs.as_mut().unwrap();
        ostd::for_all_general_regs!(write_user_reg_by_offset, regs, offset, value);

        unreachable!("the offset is valid in `c_user_regs_struct`")
    }

    fn set_options(&self, options: PtraceOptions) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        state.options = options;
        Ok(())
    }

    fn get_eventmsg(&self) -> Result<usize> {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        let msg = state.event.as_ref().map(|event| event.message());
        Ok(msg.unwrap_or(0))
    }

    fn get_siginfo(&self) -> Result<siginfo_t> {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        Ok(state.siginfo.get().unwrap())
    }
}

struct TraceeState {
    tracer: Weak<Thread>,
    /// The siginfo of the signal that stopped the tracee.
    siginfo: StopSigInfo,
    /// The extra message of a ptrace-event-stop.
    event: Option<PtraceEvent>,
    /// The general-purpose registers of the tracee at the time of ptrace-stop.
    #[cfg(target_arch = "x86_64")]
    general_regs: Option<GeneralRegs>,
    /// The configured ptrace options.
    options: PtraceOptions,
}

impl TraceeState {
    fn new() -> Self {
        Self {
            tracer: Weak::new(),
            siginfo: StopSigInfo::default(),
            event: None,
            #[cfg(target_arch = "x86_64")]
            general_regs: None,
            options: PtraceOptions::empty(),
        }
    }

    fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracer.upgrade()
    }
}
