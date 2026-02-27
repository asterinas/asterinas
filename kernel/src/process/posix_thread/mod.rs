// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use aster_rights::{ReadDupOp, ReadOp, WriteOp};
use ostd::{
    sync::{RoArc, RwMutexReadGuard, Waker},
    task::Task,
};

use super::{
    Credentials, Process,
    signal::{sig_mask::AtomicSigMask, sig_num::SigNum, sig_queues::SigQueues, signals::Signal},
};
use crate::{
    events::IoEvents,
    fs::{file_table::FileTable, thread_info::ThreadFsInfo},
    prelude::*,
    process::{
        Pid,
        namespace::nsproxy::NsProxy,
        signal::{PauseReason, PollHandle},
    },
    thread::{Thread, Tid},
    time::{Timer, TimerManager, clocks::ProfClock, timer::TimerGuard},
};

mod builder;
mod exit;
pub mod futex;
mod name;
mod posix_thread_ext;
pub mod ptrace;
mod robust_list;
mod thread_local;
pub mod thread_table;

pub use builder::PosixThreadBuilder;
pub(super) use exit::sigkill_other_threads;
pub use exit::{do_exit, do_exit_group};
pub use name::{MAX_THREAD_NAME_LEN, ThreadName};
pub use posix_thread_ext::AsPosixThread;
pub use robust_list::RobustListHead;
pub use thread_local::{AsThreadLocal, FileTableRefMut, ThreadLocal};

pub struct PosixThread {
    // Immutable part
    process: Weak<Process>,
    task: Weak<Task>,

    // Mutable part
    tid: AtomicU32,

    name: Mutex<ThreadName>,

    /// Process credentials. At the kernel level, credentials are a per-thread attribute.
    credentials: Credentials,

    /// The file system information of the thread.
    fs: RwMutex<Arc<ThreadFsInfo>>,

    // Files
    /// File table
    file_table: Mutex<Option<RoArc<FileTable>>>,

    // Signal
    /// Blocked signals
    sig_mask: AtomicSigMask,
    /// Thread-directed sigqueue
    sig_queues: SigQueues,
    /// The per-thread signal [`Waker`], which will be used to wake up the thread
    /// when enqueuing a signal, along with the reason why the thread is paused.
    signalled_waker: SpinLock<Option<(Arc<Waker>, PauseReason)>>,

    /// A profiling clock measures the user CPU time and kernel CPU time in the thread.
    prof_clock: Arc<ProfClock>,

    /// A manager that manages timers based on the user CPU time of the current thread.
    virtual_timer_manager: Arc<TimerManager>,

    /// A manager that manages timers based on the profiling clock of the current thread.
    prof_timer_manager: Arc<TimerManager>,

    /// I/O Scheduling priority value
    io_priority: AtomicU32,

    /// The namespaces that the thread belongs to.
    ns_proxy: Mutex<Option<Arc<NsProxy>>>,

    /// The current timer slack value for this thread.
    timer_slack_ns: AtomicU64,
    /// The default timer slack value for this thread.
    default_timer_slack_ns: AtomicU64,
}

impl PosixThread {
    pub fn process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }

    pub fn weak_process(&self) -> &Weak<Process> {
        &self.process
    }

    /// Returns the thread id
    pub fn tid(&self) -> Tid {
        self.tid.load(Ordering::Relaxed)
    }

    /// Sets the thread as the main thread by changing its thread ID.
    pub(super) fn set_main(&self, pid: Pid) {
        debug_assert_eq!(pid, self.process.upgrade().unwrap().pid());
        debug_assert_ne!(pid, self.tid.load(Ordering::Relaxed));

        self.tid.store(pid, Ordering::Relaxed);
    }

    pub fn thread_name(&self) -> &Mutex<ThreadName> {
        &self.name
    }

    /// Returns a read guard to the filesystem information of the thread.
    pub fn read_fs(&self) -> RwMutexReadGuard<'_, Arc<ThreadFsInfo>> {
        self.fs.read()
    }

    /// Sets the filesystem information of the thread.
    pub(in crate::process) fn set_fs(&self, new_fs: Arc<ThreadFsInfo>) {
        let mut fs_lock = self.fs.write();
        *fs_lock = new_fs;
    }

    pub fn file_table(&self) -> &Mutex<Option<RoArc<FileTable>>> {
        &self.file_table
    }

    /// Gets the reference to the signal mask of the thread.
    ///
    /// Note that while this function offers mutable access to the signal mask,
    /// it is not sound for callers other than the current thread to modify the
    /// signal mask. They may only read the signal mask.
    pub fn sig_mask(&self) -> &AtomicSigMask {
        &self.sig_mask
    }

    pub(super) fn sig_queues(&self) -> &SigQueues {
        &self.sig_queues
    }

    /// Returns whether the signal is blocked by the thread.
    pub fn has_signal_blocked(&self, signum: SigNum) -> bool {
        // FIXME: Some signals cannot be blocked, even set in sig_mask.
        self.sig_mask.contains(signum, Ordering::Relaxed)
    }

    /// Sets the input [`Waker`] as the signalled waker of this thread,
    /// along with the reason why the thread is paused.
    ///
    /// This approach can collaborate with signal-aware wait methods.
    /// Once a signalled waker is set for a thread, it cannot be reset until it is cleared.
    ///
    /// # Panics
    ///
    /// If setting a new waker before clearing the current thread's signalled waker
    /// this method will panic.
    pub fn set_signalled_waker(&self, waker: Arc<Waker>, reason: PauseReason) {
        let mut signalled_waker = self.signalled_waker.lock();
        assert!(signalled_waker.is_none());
        *signalled_waker = Some((waker, reason));
    }

    /// Clears the signalled waker of this thread.
    pub fn clear_signalled_waker(&self) {
        *self.signalled_waker.lock() = None;
    }

    /// Returns the sleeping state of this thread.
    pub fn sleeping_state(&self) -> SleepingState {
        // This implementation prevents a thread (let's call it `threadA`) that is
        // sleeping in an interruptible wait from being mistakenly reported as
        // sleeping in an uninterruptible wait due to a race condition, where another
        // thread (`threadB`) may observe that its `task.schedule_info().cpu` is
        // `AtomicCpuId::NONE` and its `signalled_waker` is `None` (not set yet or
        // already cleared).
        //
        // When `threadA` enters an interruptible wait, it executes the following steps:
        // ```
        // A1: Acquire signalled_waker.lock |
        // A2: set signalled_waker to Some  |-- critical section #1
        // A3: Release signalled_waker.lock |
        // A4: cpu.set_to_none(Relaxed)
        // A5: cpu.set_if_is_none(cpuid, Relaxed)
        // A6: Acquire signalled_waker.lock |
        // A7: set signalled_waker to None  |-- critical section #2
        // A8: Release signalled_waker.lock |
        // ```
        //
        // When `threadB` calls `threadA.sleeping_state()`, it executes the following steps:
        // ```
        // B1: Acquire threadA.signalled_waker.lock |
        // B2: check threadA.signalled_waker        |-- critical section #3
        // B3: check threadA.cpu.get(Relaxed)       |
        // B4: Release threadA.signalled_waker.lock |
        // ```
        //
        // We can see that:
        //  - If #3 happens before #1, B3 can not observe the effect of A4 due to the
        //    release-acquire pair B4-A1.
        //  - If #3 happens between #1 and #2, B2 will always see a `Some`.
        //  - If #3 happens after #2, B3 can observe the effect of A5 due to the
        //    release-acquire pair A8-B1.
        // Therefore, the condition where both B2 and B3 see `None` will never happen.
        //
        // Similarly, this implementation prevents a process that has been stopped by
        // a signal or ptrace from being incorrectly reported as sleeping in an
        // (un)interruptible wait.
        //
        // FIXME: This implementation cannot prevent a stopped process from being
        // reported as running when `crate::process::signal::handle_pending_signal`
        // is called, but the pending signal is not a `SIGCONT`. However, is this
        // actually a problem? We considered an approach to fix this issue, but it
        // does not fully resolve it and has some drawbacks. For more details, see
        // <https://github.com/asterinas/asterinas/pull/2491#issuecomment-3527958970>.
        let signalled_waker = self.signalled_waker.lock();
        let task = self.task.upgrade().unwrap();
        match (
            signalled_waker.as_ref(),
            task.schedule_info().cpu.get().is_none(),
        ) {
            (Some((_, PauseReason::Sleep)), true) => SleepingState::Interruptible,
            (Some((_, PauseReason::StopBySignal)), true) => SleepingState::StopBySignal,
            (Some((_, PauseReason::StopByPtrace)), true) => SleepingState::StopByPtrace,
            (None, true) => SleepingState::Uninterruptible,
            (_, false) => SleepingState::Running,
        }
    }

    /// Wakes up the signalled waker.
    pub fn wake_signalled_waker(&self) {
        if let Some((waker, _)) = &*self.signalled_waker.lock() {
            waker.wake_up();
        }
    }

    /// Enqueues a thread-directed signal.
    ///
    /// This method does not perform permission checks on user signals.
    /// Therefore, unless the caller can ensure that there are no permission issues,
    /// this method should be used to enqueue kernel signals or fault signals.
    pub fn enqueue_signal(&self, signal: Box<dyn Signal>) {
        self.sig_queues.enqueue(signal);
        self.wake_signalled_waker();
    }

    pub fn register_signalfd_poller(&self, poller: &mut PollHandle, mask: IoEvents) {
        self.sig_queues.register_signalfd_poller(poller, mask);
        self.process()
            .sig_queues()
            .register_signalfd_poller(poller, mask);
    }

    /// Returns a reference to the profiling clock of the current thread.
    pub fn prof_clock(&self) -> &Arc<ProfClock> {
        &self.prof_clock
    }

    /// Creates a timer based on the profiling CPU clock of the current thread.
    pub fn create_prof_timer<F>(&self, func: F) -> Arc<Timer>
    where
        F: Fn(TimerGuard) + Send + Sync + 'static,
    {
        self.prof_timer_manager.create_timer(func)
    }

    /// Creates a timer based on the user CPU clock of the current thread.
    pub fn create_virtual_timer<F>(&self, func: F) -> Arc<Timer>
    where
        F: Fn(TimerGuard) + Send + Sync + 'static,
    {
        self.virtual_timer_manager.create_timer(func)
    }

    /// Checks the `TimerCallback`s that are managed by the `prof_timer_manager`.
    /// If any have timed out, call the corresponding callback functions.
    pub fn process_expired_timers(&self) {
        self.prof_timer_manager.process_expired_timers();
    }

    /// Gets the read-only credentials of the thread.
    pub fn credentials(&self) -> Credentials<ReadOp> {
        self.credentials.dup().restrict()
    }

    /// Gets the duplicatable read-only credentials of the thread.
    pub fn credentials_dup(&self) -> Credentials<ReadDupOp> {
        self.credentials.dup().restrict()
    }

    /// Gets the write-only credentials of the current thread.
    ///
    /// It is illegal to mutate the credentials from a thread other than the
    /// current thread. For performance reasons, this function only checks it
    /// using debug assertions.
    pub fn credentials_mut(&self) -> Credentials<WriteOp> {
        debug_assert!(core::ptr::eq(
            current_thread!().as_posix_thread().unwrap(),
            self
        ));
        self.credentials.dup().restrict()
    }

    /// Returns the I/O priority value of the thread.
    pub fn io_priority(&self) -> &AtomicU32 {
        &self.io_priority
    }

    /// Returns the namespaces which the thread belongs to.
    pub fn ns_proxy(&self) -> &Mutex<Option<Arc<NsProxy>>> {
        &self.ns_proxy
    }

    /// Returns the current timer slack value in nanoseconds.
    pub fn timer_slack_ns(&self) -> u64 {
        self.timer_slack_ns.load(Ordering::Relaxed)
    }

    /// Sets the current timer slack value in nanoseconds.
    pub fn set_timer_slack_ns(&self, slack_ns: u64) {
        self.timer_slack_ns.store(slack_ns, Ordering::Relaxed);
    }

    /// Resets the current timer slack to the default value.
    pub fn reset_timer_slack_to_default(&self) {
        let default = self.default_timer_slack_ns.load(Ordering::Relaxed);
        self.timer_slack_ns.store(default, Ordering::Relaxed);
    }
}

static POSIX_TID_ALLOCATOR: AtomicU32 = AtomicU32::new(1);

/// Allocates a new tid for the new posix thread
pub fn allocate_posix_tid() -> Tid {
    let tid = POSIX_TID_ALLOCATOR.fetch_add(1, Ordering::SeqCst);
    if tid >= PID_MAX {
        // When the kernel's next PID value reaches `PID_MAX`,
        // it should wrap back to a minimum PID value.
        // PIDs with a value of `PID_MAX` or larger should not be allocated.
        // Reference: <https://docs.kernel.org/admin-guide/sysctl/kernel.html#pid-max>.
        //
        // FIXME: Currently, we cannot determine which PID is recycled,
        // so we are unable to allocate smaller PIDs.
        warn!("the allocated ID is greater than the maximum allowed PID");
    }
    tid
}

/// Returns the last allocated tid
pub fn last_tid() -> Tid {
    POSIX_TID_ALLOCATOR.load(Ordering::SeqCst) - 1
}

/// The maximum allowed process ID.
//
// FIXME: The current value is chosen arbitrarily.
// This value can be modified by the user by writing to `/proc/sys/kernel/pid_max`.
pub const PID_MAX: u32 = u32::MAX / 2;

/// The sleeping state of a thread.
#[derive(Debug, Clone, Copy)]
pub enum SleepingState {
    /// The thread is running.
    Running,
    /// The thread is sleeping in an interruptible wait.
    Interruptible,
    /// The thread is sleeping in an uninterruptible wait.
    Uninterruptible,
    /// The thread is stopped by a signal.
    StopBySignal,
    /// The thread is stopped by ptrace.
    StopByPtrace,
}
