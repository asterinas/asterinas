// SPDX-License-Identifier: MPL-2.0

//! Ptrace implementation for POSIX threads.

use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "x86_64")]
use ostd::arch::cpu::context::GeneralRegs;
use ostd::{arch::cpu::context::UserContext, sync::Waiter};

use super::{AsPosixThread, PosixThread};
#[cfg(target_arch = "x86_64")]
use crate::{arch::ptrace as arch_ptrace, process::posix_thread::NOT_A_SYSCALL};
use crate::{
    prelude::*,
    process::{
        CloneArgs, CloneFlags, WaitOptions,
        signal::{
            DequeuedSignal, PauseReason,
            c_types::siginfo_t,
            constants::{CLD_TRAPPED, SIGCHLD, SIGKILL, SIGTRAP},
            signals::{kernel::KernelSignal, raw::RawSignal, user::UserSignal},
        },
    },
    thread::{Thread, Tid},
};

mod util;

use util::StopDeliverySignal;
pub use util::{PtraceContRequest, PtraceOptions, PtraceWaitStatus};
pub(in crate::process) use util::{PtraceEvent, PtraceStopResult};

impl PosixThread {
    /// Returns whether this thread is being traced.
    pub(in crate::process) fn is_traced(&self) -> bool {
        self.tracer().is_some()
    }

    /// Returns the tracer of this thread if it is being traced.
    pub fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracee_status.get().and_then(|status| status.tracer())
    }

    /// Sets the tracer of this thread.
    ///
    /// # Errors
    ///
    /// Returns `EPERM` if this thread is already being traced.
    fn set_tracer(&self, tracer: Weak<Thread>) -> Result<()> {
        let status = self.tracee_status.call_once(TraceeStatus::new);
        status.set_tracer(tracer)
    }

    /// Detaches the tracer of this thread.
    pub(in crate::process) fn detach_tracer(&self) {
        self.detach_tracer_with(|_| {});
    }

    /// Detaches the tracer of this thread with a callback.
    fn detach_tracer_with<F>(&self, detach_callback: F)
    where
        F: FnOnce(&TraceeState),
    {
        if let Some(status) = self.tracee_status.get() {
            status.detach_tracer_with(detach_callback);
            self.wake_signalled_waker();
        }
    }

    /// Stops this thread by ptrace with the given signal if it is currently traced.
    ///
    /// Returns a [`PtraceStopResult`] indicating why this ptrace-stop ended.
    pub(in crate::process) fn ptrace_stop(
        &self,
        signal: DequeuedSignal,
        ctx: &Context,
        user_ctx: &mut UserContext,
    ) -> PtraceStopResult {
        if let Some(status) = self.tracee_status.get() {
            status.ptrace_stop(signal, ctx, user_ctx)
        } else {
            PtraceStopResult::NotTraced(signal)
        }
    }

    /// Stops this thread by ptrace on the `event` if it is currently traced,
    /// and the corresponding option is enabled.
    ///
    /// May block in the event-stop until the tracer continues the stop,
    /// or until a `SIGKILL` interrupts it.
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

    /// Returns the ptrace-stop status changes for the `wait` syscall.
    pub(in crate::process) fn wait_ptrace_stopped(
        &self,
        options: WaitOptions,
    ) -> Option<PtraceWaitStatus> {
        self.tracee_status
            .get()
            .and_then(|status| status.wait(options))
    }

    /// Continues this thread from a ptrace-stop.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    pub fn ptrace_continue(&self, request: PtraceContRequest, ctx: &Context) -> Result<()> {
        let status = self.get_tracee_status()?;

        status.resume(request, ctx)?;
        self.wake_signalled_waker();

        Ok(())
    }

    /// Gets the general-purpose registers of this thread for ptrace.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_get_regs(&self) -> Result<arch_ptrace::CUserRegsStruct> {
        let status = self.get_tracee_status()?;
        status.get_regs()
    }

    /// Sets the general-purpose registers of this thread for ptrace.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    #[cfg(target_arch = "x86_64")]
    pub fn ptrace_set_regs(&self, regs: arch_ptrace::CUserRegsStruct) -> Result<()> {
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

    /// Gets the event of the last ptrace-stop,
    /// if it is a ptrace-event-stop.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    pub fn ptrace_get_event(&self) -> Result<Option<PtraceEvent>> {
        let status = self.get_tracee_status()?;
        status.get_event()
    }

    /// Gets the signal info of this thread for ptrace.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread is not ptrace-stopped.
    pub fn ptrace_get_siginfo(&self) -> Result<siginfo_t> {
        let status = self.get_tracee_status()?;
        status.get_siginfo()
    }

    /// Returns the tracee status of this thread if it has ever been traced.
    ///
    /// # Errors
    ///
    /// Returns `ESRCH` if this thread has never been traced.
    fn get_tracee_status(&self) -> Result<&TraceeStatus> {
        self.tracee_status
            .get()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the thread has never been traced"))
    }
}

impl PosixThread {
    /// Attaches this tracer to the given tracee.
    ///
    /// # Errors
    ///
    /// Returns `EPERM` if the tracee is already being traced.
    ///
    /// # Panics
    ///
    /// Panics if `tracer_thread` and `self` do not point to the same thread,
    /// or if `tracee_thread` is not a POSIX thread.
    pub fn attach_to(&self, tracer_thread: &Arc<Thread>, tracee_thread: Arc<Thread>) -> Result<()> {
        debug_assert!(core::ptr::eq(
            tracer_thread.as_posix_thread().unwrap(),
            self
        ));

        let tracees = self.tracees.call_once(|| Mutex::new(BTreeMap::new()));

        // Lock order: tracer.tracees -> tracee.tracee_status
        let mut tracees = tracees.lock();
        if tracer_thread.is_exited() {
            // Pretend that the tracer immediately detaches the tracee,
            // if the tracer has already exited.
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/ptrace.c#L496-L498>
            return Ok(());
        }

        let tracee = tracee_thread.as_posix_thread().unwrap();
        tracee.set_tracer(Arc::downgrade(tracer_thread))?;
        tracees.insert(tracee.tid(), tracee_thread);

        Ok(())
    }

    /// Returns the tracee map of this thread if it is a tracer.
    pub(in crate::process) fn tracees(&self) -> Option<&Mutex<BTreeMap<Tid, Arc<Thread>>>> {
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
    pub(in crate::process) fn clear_tracees(&self) {
        let Some(tracees) = self.tracees() else {
            return;
        };

        let mut tracees_to_kill = Vec::new();

        // Lock order: tracer.tracees -> tracee.tracee_status
        let tracees = tracees.lock();
        for (_, tracee_thread) in tracees.iter() {
            let tracee = tracee_thread.as_posix_thread().unwrap();
            let mut needs_kill = false;
            tracee.detach_tracer_with(|state| {
                if state.options.contains(PtraceOptions::PTRACE_O_EXITKILL) {
                    needs_kill = true;
                }
            });
            if needs_kill {
                tracees_to_kill.push(tracee_thread.clone());
            }
        }

        drop(tracees);

        for tracee in tracees_to_kill {
            let tracee = tracee.as_posix_thread().unwrap();
            tracee.enqueue_signal(Box::new(KernelSignal::new(SIGKILL)));
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

    fn detach_tracer_with<F>(&self, detach_callback: F)
    where
        F: FnOnce(&TraceeState),
    {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();

        state.tracer = Weak::new();
        #[cfg(target_arch = "x86_64")]
        {
            if let Some(regs) = state.general_regs.as_mut() {
                arch_ptrace::disable_single_step(regs);
            }
        }
        detach_callback(&state);
        self.is_stopped.store(false, Ordering::Relaxed);
    }

    fn ptrace_stop(
        &self,
        signal: DequeuedSignal,
        ctx: &Context,
        user_ctx: &mut UserContext,
    ) -> PtraceStopResult {
        #[cfg(not(target_arch = "x86_64"))]
        let _ = user_ctx;

        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();

        let Some(tracer) = state.tracer() else {
            return PtraceStopResult::NotTraced(signal);
        };

        let wait_status = PtraceWaitStatus::from_signal(signal.signal().num());

        self.do_ptrace_stop(state, tracer, signal, wait_status, None, ctx, user_ctx)
    }

    fn ptrace_may_stop_on(&self, event: PtraceEvent, ctx: &Context, user_ctx: &mut UserContext) {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();

        let Some(tracer) = state.tracer() else {
            return;
        };

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
        let signal = Box::new(RawSignal::new(siginfo));
        let signal = DequeuedSignal::FromThread(signal);
        let wait_status = PtraceWaitStatus::from_event(&event);

        self.do_ptrace_stop(
            state,
            tracer,
            signal,
            wait_status,
            Some(event),
            ctx,
            user_ctx,
        );
    }

    fn do_ptrace_stop(
        &self,
        mut state: MutexGuard<'_, TraceeState>,
        tracer: Arc<Thread>,
        signal: DequeuedSignal,
        wait_status: PtraceWaitStatus,
        event: Option<PtraceEvent>,
        ctx: &Context,
        user_ctx: &mut UserContext,
    ) -> PtraceStopResult {
        #[cfg(not(target_arch = "x86_64"))]
        let _ = user_ctx;

        debug_assert!(!self.is_ptrace_stopped());

        state.signal.stop(signal, wait_status);
        state.event = event;
        #[cfg(target_arch = "x86_64")]
        {
            state.general_regs = Some(*user_ctx.general_regs());
            state.orig_syscall_ret = ctx.posix_thread.orig_syscall_ret();
        }
        self.is_stopped.store(true, Ordering::Relaxed);
        drop(state);

        let tracer = tracer.as_posix_thread().unwrap();
        tracer.enqueue_signal(Box::new(RawSignal::new({
            let mut siginfo = siginfo_t::new(SIGCHLD, CLD_TRAPPED);
            siginfo.set_pid_uid_by(ctx);
            siginfo
        })));
        tracer.process().children_wait_queue().wake_all();

        let waiter = Waiter::new_pair().0;
        if waiter
            .pause_until_by(
                || (!self.is_ptrace_stopped()).then_some(()),
                PauseReason::StopByPtrace,
            )
            .is_err()
        {
            // A `SIGKILL` interrupts this ptrace-stop.
            let mut state = self.state.lock();
            state.signal.clear();
            state.event = None;
            #[cfg(target_arch = "x86_64")]
            {
                state.general_regs = None;
                state.orig_syscall_ret = NOT_A_SYSCALL;
            }
            self.is_stopped.store(false, Ordering::Relaxed);
            return PtraceStopResult::Interrupted;
        };

        let mut state = self.state.lock();
        let signal = state.signal.clear();
        state.event = None;

        #[cfg(target_arch = "x86_64")]
        {
            let regs = state.general_regs.take().unwrap();
            *user_ctx.general_regs_mut() = regs;
            ctx.posix_thread
                .set_orig_syscall_ret(state.orig_syscall_ret);
            state.orig_syscall_ret = NOT_A_SYSCALL;
        }

        PtraceStopResult::Continued(signal)
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

    fn wait(&self, options: WaitOptions) -> Option<PtraceWaitStatus> {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();

        // Avoid the race with `detach_tracer` or `resume` in between.
        if !self.is_ptrace_stopped() {
            return None;
        }

        state.signal.wait(options)
    }

    fn resume(&self, request: PtraceContRequest, ctx: &Context) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        if let Some(sig_num) = request.sig_num() {
            state
                .signal
                .inject(Box::new(UserSignal::new_kill(sig_num, ctx)));
        } else {
            state.signal.clear();
        }

        #[cfg(target_arch = "x86_64")]
        {
            let regs = state.general_regs.as_mut().unwrap();
            if matches!(request, PtraceContRequest::SingleStep(_)) {
                arch_ptrace::enable_single_step(regs);
            } else {
                arch_ptrace::disable_single_step(regs);
            }
        }

        self.is_stopped.store(false, Ordering::Relaxed);

        Ok(())
    }

    #[cfg(target_arch = "x86_64")]
    fn get_regs(&self) -> Result<arch_ptrace::CUserRegsStruct> {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        let regs = state.general_regs.as_ref().unwrap();
        let mut regs = arch_ptrace::CUserRegsStruct::from(regs);
        regs.orig_rax = state.orig_syscall_ret;
        Ok(regs)
    }

    #[cfg(target_arch = "x86_64")]
    fn set_regs(&self, regs: arch_ptrace::CUserRegsStruct) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        regs.apply_to(state.general_regs.as_mut().unwrap())?;
        state.orig_syscall_ret = regs.orig_rax;

        Ok(())
    }

    #[cfg(target_arch = "x86_64")]
    fn peek_user(&self, offset: usize) -> Result<usize> {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();
        self.check_ptrace_stopped(&state)?;
        let regs = state.general_regs.as_ref().unwrap();
        arch_ptrace::read_user_word(regs, state.orig_syscall_ret, offset)
    }

    #[cfg(target_arch = "x86_64")]
    fn poke_user(&self, offset: usize, value: usize) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();
        self.check_ptrace_stopped(&state)?;
        let mut orig_syscall_ret = state.orig_syscall_ret;
        let regs = state.general_regs.as_mut().unwrap();
        arch_ptrace::write_user_word(regs, &mut orig_syscall_ret, offset, value)?;
        state.orig_syscall_ret = orig_syscall_ret;
        Ok(())
    }

    fn set_options(&self, options: PtraceOptions) -> Result<()> {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        state.options = options;
        Ok(())
    }

    fn get_event(&self) -> Result<Option<PtraceEvent>> {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        Ok(state.event.clone())
    }

    fn get_siginfo(&self) -> Result<siginfo_t> {
        // Hold the lock first to avoid race conditions.
        let state = self.state.lock();
        self.check_ptrace_stopped(&state)?;

        Ok(state.signal.get().unwrap().to_info())
    }
}

struct TraceeState {
    tracer: Weak<Thread>,
    /// The signal associated with the current ptrace-stop and later signal delivery.
    signal: StopDeliverySignal,
    /// The event associated with the current ptrace-event-stop.
    event: Option<PtraceEvent>,
    /// The configured ptrace options.
    options: PtraceOptions,
    /// The general-purpose registers of the tracee at the time of ptrace-stop.
    #[cfg(target_arch = "x86_64")]
    general_regs: Option<GeneralRegs>,
    /// The value of `PosixThread::orig_syscall_ret` at the time of ptrace-stop.
    #[cfg(target_arch = "x86_64")]
    orig_syscall_ret: usize,
}

impl TraceeState {
    fn new() -> Self {
        Self {
            tracer: Weak::new(),
            signal: StopDeliverySignal::default(),
            event: None,
            options: PtraceOptions::empty(),
            #[cfg(target_arch = "x86_64")]
            general_regs: None,
            #[cfg(target_arch = "x86_64")]
            orig_syscall_ret: NOT_A_SYSCALL,
        }
    }

    fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracer.upgrade()
    }
}
