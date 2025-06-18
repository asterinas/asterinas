// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use aster_rights::{ReadDupOp, ReadOp, WriteOp};
use ostd::sync::{RoArc, Waker};

use super::{
    kill::SignalSenderIds,
    signal::{
        sig_disposition::SigDispositions,
        sig_mask::{AtomicSigMask, SigMask, SigSet},
        sig_num::SigNum,
        sig_queues::SigQueues,
        signals::Signal,
        SigEvents, SigEventsFilter,
    },
    Credentials, Process,
};
use crate::{
    events::Observer,
    fs::{file_table::FileTable, thread_info::ThreadFsInfo},
    prelude::*,
    process::signal::constants::SIGCONT,
    thread::{Thread, Tid},
    time::{clocks::ProfClock, Timer, TimerManager},
};

mod builder;
mod exit;
pub mod futex;
mod name;
mod posix_thread_ext;
mod robust_list;
mod thread_local;
pub mod thread_table;

pub use builder::PosixThreadBuilder;
pub use exit::{do_exit, do_exit_group};
pub use name::{ThreadName, MAX_THREAD_NAME_LEN};
pub use posix_thread_ext::AsPosixThread;
pub use robust_list::RobustListHead;
pub use thread_local::{AsThreadLocal, FileTableRefMut, ThreadLocal};

pub struct PosixThread {
    // Immutable part
    process: Weak<Process>,
    tid: Tid,

    // Mutable part
    name: Mutex<Option<ThreadName>>,

    /// Process credentials. At the kernel level, credentials are a per-thread attribute.
    credentials: Credentials,

    // Files
    /// File table
    file_table: Mutex<Option<RoArc<FileTable>>>,
    /// File system
    fs: Arc<ThreadFsInfo>,

    // Signal
    /// Blocked signals
    sig_mask: AtomicSigMask,
    /// Thread-directed sigqueue
    sig_queues: SigQueues,
    /// The per-thread signal [`Waker`], which will be used to wake up the thread
    /// when enqueuing a signal.
    signalled_waker: SpinLock<Option<Arc<Waker>>>,

    /// A profiling clock measures the user CPU time and kernel CPU time in the thread.
    prof_clock: Arc<ProfClock>,

    /// A manager that manages timers based on the user CPU time of the current thread.
    virtual_timer_manager: Arc<TimerManager>,

    /// A manager that manages timers based on the profiling clock of the current thread.
    prof_timer_manager: Arc<TimerManager>,

    /// I/O Scheduling priority value
    io_priority: AtomicU32,
}

impl PosixThread {
    pub fn process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }

    pub fn weak_process(&self) -> Weak<Process> {
        Weak::clone(&self.process)
    }

    /// Returns the thread id
    pub fn tid(&self) -> Tid {
        self.tid
    }

    pub fn thread_name(&self) -> &Mutex<Option<ThreadName>> {
        &self.name
    }

    pub fn file_table(&self) -> &Mutex<Option<RoArc<FileTable>>> {
        &self.file_table
    }

    pub fn fs(&self) -> &Arc<ThreadFsInfo> {
        &self.fs
    }

    /// Get the reference to the signal mask of the thread.
    ///
    /// Note that while this function offers mutable access to the signal mask,
    /// it is not sound for callers other than the current thread to modify the
    /// signal mask. They may only read the signal mask.
    pub fn sig_mask(&self) -> &AtomicSigMask {
        &self.sig_mask
    }

    pub fn sig_pending(&self) -> SigSet {
        self.sig_queues.sig_pending()
    }

    /// Returns whether the thread has some pending signals
    /// that are not blocked.
    pub fn has_pending(&self) -> bool {
        let blocked = self.sig_mask().load(Ordering::Relaxed);
        self.sig_queues.has_pending(blocked)
    }

    /// Returns whether the signal is blocked by the thread.
    pub(in crate::process) fn has_signal_blocked(&self, signum: SigNum) -> bool {
        // FIXME: Some signals cannot be blocked, even set in sig_mask.
        self.sig_mask.contains(signum, Ordering::Relaxed)
    }

    /// Checks whether the signal can be delivered to the thread.
    ///
    /// For a signal can be delivered to the thread, the sending thread must either
    /// be privileged, or the real or effective user ID of the sending thread must equal
    /// the real or saved set-user-ID of the target thread.
    ///
    /// For SIGCONT, the sending and receiving processes should belong to the same session.
    pub(in crate::process) fn check_signal_perm(
        &self,
        signum: Option<&SigNum>,
        sender: &SignalSenderIds,
    ) -> Result<()> {
        if sender.euid().is_root() {
            return Ok(());
        }

        if let Some(signum) = signum
            && *signum == SIGCONT
        {
            let receiver_sid = self.process().sid();
            if receiver_sid == sender.sid().unwrap() {
                return Ok(());
            }

            return_errno_with_message!(
                Errno::EPERM,
                "sigcont requires that sender and receiver belongs to the same session"
            );
        }

        let (receiver_ruid, receiver_suid) = {
            let credentials = self.credentials();
            (credentials.ruid(), credentials.suid())
        };

        // FIXME: further check the below code to ensure the behavior is same as Linux. According
        // to man(2) kill, the real or effective user ID of the sending process must equal the
        // real or saved set-user-ID of the target process.
        if sender.ruid() == receiver_ruid
            || sender.ruid() == receiver_suid
            || sender.euid() == receiver_ruid
            || sender.euid() == receiver_suid
        {
            return Ok(());
        }

        return_errno_with_message!(Errno::EPERM, "sending signal to the thread is not allowed.");
    }

    /// Sets the input [`Waker`] as the signalled waker of this thread.
    ///
    /// This approach can collaborate with signal-aware wait methods.
    /// Once a signalled waker is set for a thread, it cannot be reset until it is cleared.
    ///
    /// # Panics
    ///
    /// If setting a new waker before clearing the current thread's signalled waker
    /// this method will panic.
    pub fn set_signalled_waker(&self, waker: Arc<Waker>) {
        let mut signalled_waker = self.signalled_waker.lock();
        assert!(signalled_waker.is_none());
        *signalled_waker = Some(waker);
    }

    /// Clears the signalled waker of this thread.
    pub fn clear_signalled_waker(&self) {
        *self.signalled_waker.lock() = None;
    }

    /// Wakes up the signalled waker.
    pub fn wake_signalled_waker(&self) {
        if let Some(waker) = &*self.signalled_waker.lock() {
            waker.wake_up();
        }
    }

    /// Enqueues a thread-directed signal.
    ///
    /// This method does not perform permission checks on user signals. Therefore, unless the
    /// caller can ensure that there are no permission issues, this method should be used for
    /// enqueue kernel signals or fault signals.
    pub fn enqueue_signal(&self, signal: Box<dyn Signal>) {
        let process = self.process();
        let sig_dispositions = process.sig_dispositions().lock();

        let signum = signal.num();
        if sig_dispositions.get(signum).will_ignore(signum) {
            return;
        }

        self.enqueue_signal_locked(signal, sig_dispositions);
    }

    /// Enqueues a thread-directed signal with locked dispositions.
    ///
    /// By locking dispositions, the caller should have already checked the signal is not to be
    /// ignored.
    //
    // FIXME: According to Linux behavior, we should enqueue ignored signals blocked by all
    // threads, as a thread may change the signal handler and unblock them in the future. However,
    // achieving this behavior properly without maintaining a process-wide signal queue is
    // difficult. For instance, if we randomly select a thread-wide signal queue, the thread that
    // modifies the signal handler and unblocks the signal may not be the same one. Consequently,
    // the current implementation uses a simpler mechanism that never enqueues any ignored signals.
    pub(in crate::process) fn enqueue_signal_locked(
        &self,
        signal: Box<dyn Signal>,
        _sig_dispositions: MutexGuard<SigDispositions>,
    ) {
        self.sig_queues.enqueue(signal);
        self.wake_signalled_waker();
    }

    /// Returns a reference to the profiling clock of the current thread.
    pub fn prof_clock(&self) -> &Arc<ProfClock> {
        &self.prof_clock
    }

    /// Creates a timer based on the profiling CPU clock of the current thread.
    pub fn create_prof_timer<F>(&self, func: F) -> Arc<Timer>
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.prof_timer_manager.create_timer(func)
    }

    /// Creates a timer based on the user CPU clock of the current thread.
    pub fn create_virtual_timer<F>(&self, func: F) -> Arc<Timer>
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.virtual_timer_manager.create_timer(func)
    }

    /// Checks the `TimerCallback`s that are managed by the `prof_timer_manager`.
    /// If any have timed out, call the corresponding callback functions.
    pub fn process_expired_timers(&self) {
        self.prof_timer_manager.process_expired_timers();
    }

    pub fn dequeue_signal(&self, mask: &SigMask) -> Option<Box<dyn Signal>> {
        self.sig_queues.dequeue(mask)
    }

    pub fn register_sigqueue_observer(
        &self,
        observer: Weak<dyn Observer<SigEvents>>,
        filter: SigEventsFilter,
    ) {
        self.sig_queues.register_observer(observer, filter);
    }

    pub fn unregister_sigqueue_observer(&self, observer: &Weak<dyn Observer<SigEvents>>) {
        self.sig_queues.unregister_observer(observer);
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
