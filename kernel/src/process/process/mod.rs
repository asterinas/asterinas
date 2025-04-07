// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use self::timer_manager::PosixTimerManager;
use super::{
    posix_thread::AsPosixThread,
    process_table,
    process_vm::{InitStackReader, ProcessVm, ProcessVmarGuard, ProgramBreak},
    rlimit::ResourceLimits,
    signal::{
        sig_disposition::SigDispositions,
        sig_num::{AtomicSigNum, SigNum},
        signals::Signal,
    },
    status::ProcessStatus,
    task_set::TaskSet,
};
use crate::{
    prelude::*,
    process::{signal::Pollee, status::StopWaitStatus, WaitOptions},
    sched::{AtomicNice, Nice},
    thread::{AsThread, Thread},
    time::clocks::ProfClock,
};

mod init_proc;
mod job_control;
mod process_group;
mod session;
mod terminal;
mod timer_manager;

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
pub use init_proc::spawn_init_process;
pub use job_control::JobControl;
use ostd::{sync::WaitQueue, task::Task};
pub use process_group::ProcessGroup;
pub use session::Session;
pub use terminal::Terminal;

/// Process id.
pub type Pid = u32;
define_atomic_version_of_integer_like_type!(Pid, {
    #[derive(Debug)]
    pub struct AtomicPid(AtomicU32);
});
/// Process group id.
pub type Pgid = u32;
/// Session Id.
pub type Sid = u32;

pub type ExitCode = u32;

pub(super) fn init() {
    timer_manager::init();
}

/// Process stands for a set of threads that shares the same userspace.
pub struct Process {
    // Immutable Part
    pid: Pid,

    process_vm: ProcessVm,
    /// Wait for child status changed
    children_wait_queue: WaitQueue,
    pub(super) pidfile_pollee: Pollee,

    // Mutable Part
    /// The executable path.
    executable_path: RwLock<String>,
    /// The threads
    tasks: Mutex<TaskSet>,
    /// Process status
    status: ProcessStatus,
    /// Parent process
    pub(super) parent: ParentProcess,
    /// Children processes
    children: Mutex<BTreeMap<Pid, Arc<Process>>>,
    /// Process group
    pub(super) process_group: Mutex<Weak<ProcessGroup>>,
    /// resource limits
    resource_limits: ResourceLimits,
    /// Scheduling priority nice value
    /// According to POSIX.1, the nice value is a per-process attribute,
    /// the threads in a process should share a nice value.
    nice: AtomicNice,

    // Child reaper attribute
    /// Whether the process is a child subreaper.
    ///
    /// A subreaper can be considered as a sort of "sub-init".
    /// Instead of letting the init process to reap all orphan zombie processes,
    /// a subreaper can reap orphan zombie processes among its descendants.
    is_child_subreaper: AtomicBool,

    /// Whether the process has a subreaper that will reap it when the
    /// process becomes orphaned.
    ///
    /// If `has_child_subreaper` is true in a `Process`, this attribute should
    /// also be true for all of its descendants.
    pub(super) has_child_subreaper: AtomicBool,

    // Signal
    /// Sig dispositions
    sig_dispositions: Arc<Mutex<SigDispositions>>,
    /// The signal that the process should receive when parent process exits.
    parent_death_signal: AtomicSigNum,

    /// The signal that should be sent to the parent when this process exits.
    exit_signal: AtomicSigNum,

    /// A profiling clock measures the user CPU time and kernel CPU time of the current process.
    prof_clock: Arc<ProfClock>,

    /// A manager that manages timer resources and utilities of the process.
    timer_manager: PosixTimerManager,
}

/// Representing a parent process by holding a weak reference to it and its PID.
///
/// This type caches the value of the PID so that it can be retrieved cheaply.
///
/// The benefit of using `ParentProcess` over `(Mutex<Weak<Process>>, AtomicPid,)` is to
/// enforce the invariant that the cached PID and the weak reference are always kept in sync.
pub struct ParentProcess {
    process: Mutex<Weak<Process>>,
    pid: AtomicPid,
}

impl ParentProcess {
    pub fn new(process: Weak<Process>) -> Self {
        let pid = match process.upgrade() {
            Some(process) => process.pid(),
            None => 0,
        };

        Self {
            process: Mutex::new(process),
            pid: AtomicPid::new(pid),
        }
    }

    pub fn pid(&self) -> Pid {
        self.pid.load(Ordering::Relaxed)
    }

    pub fn lock(&self) -> ParentProcessGuard<'_> {
        ParentProcessGuard {
            guard: self.process.lock(),
            this: self,
        }
    }
}

pub struct ParentProcessGuard<'a> {
    guard: MutexGuard<'a, Weak<Process>>,
    this: &'a ParentProcess,
}

impl ParentProcessGuard<'_> {
    pub fn process(&self) -> Weak<Process> {
        self.guard.clone()
    }

    pub fn pid(&self) -> Pid {
        self.this.pid()
    }

    /// Update both pid and weak ref.
    pub fn set_process(&mut self, new_process: &Arc<Process>) {
        self.this.pid.store(new_process.pid(), Ordering::Relaxed);
        *self.guard = Arc::downgrade(new_process);
    }
}

impl Process {
    /// Returns the current process.
    ///
    /// It returns `None` if:
    ///  - the function is called in the bootstrap context;
    ///  - or if the current task is not associated with a process.
    pub fn current() -> Option<Arc<Process>> {
        Some(Task::current()?.as_posix_thread()?.process())
    }

    pub(super) fn new(
        pid: Pid,
        parent: Weak<Process>,
        executable_path: String,
        process_vm: ProcessVm,

        resource_limits: ResourceLimits,
        nice: Nice,
        sig_dispositions: Arc<Mutex<SigDispositions>>,
    ) -> Arc<Self> {
        // SIGCHID does not interrupt pauser. Child process will
        // resume paused parent when doing exit.
        let children_wait_queue = WaitQueue::new();

        let prof_clock = ProfClock::new();

        Arc::new_cyclic(|process_ref: &Weak<Process>| Self {
            pid,
            tasks: Mutex::new(TaskSet::new()),
            executable_path: RwLock::new(executable_path),
            process_vm,
            children_wait_queue,
            pidfile_pollee: Pollee::new(),
            status: ProcessStatus::default(),
            parent: ParentProcess::new(parent),
            children: Mutex::new(BTreeMap::new()),
            process_group: Mutex::new(Weak::new()),
            is_child_subreaper: AtomicBool::new(false),
            has_child_subreaper: AtomicBool::new(false),
            sig_dispositions,
            parent_death_signal: AtomicSigNum::new_empty(),
            exit_signal: AtomicSigNum::new_empty(),
            resource_limits,
            nice: AtomicNice::new(nice),
            timer_manager: PosixTimerManager::new(&prof_clock, process_ref),
            prof_clock,
        })
    }

    /// Runs the process.
    pub(super) fn run(&self) {
        let tasks = self.tasks.lock();
        // when run the process, the process should has only one thread
        debug_assert!(tasks.as_slice().len() == 1);
        debug_assert!(!self.status().is_zombie());
        let task = tasks.main().clone();
        // should not hold the lock when run thread
        drop(tasks);
        let thread = task.as_thread().unwrap();
        thread.run();
    }

    // *********** Basic structures ***********

    pub fn pid(&self) -> Pid {
        self.pid
    }

    /// Gets the profiling clock of the process.
    pub fn prof_clock(&self) -> &Arc<ProfClock> {
        &self.prof_clock
    }

    /// Gets the timer resources and utilities of the process.
    pub fn timer_manager(&self) -> &PosixTimerManager {
        &self.timer_manager
    }

    pub fn tasks(&self) -> &Mutex<TaskSet> {
        &self.tasks
    }

    pub fn executable_path(&self) -> String {
        self.executable_path.read().clone()
    }

    pub fn set_executable_path(&self, executable_path: String) {
        *self.executable_path.write() = executable_path;
    }

    pub fn resource_limits(&self) -> &ResourceLimits {
        &self.resource_limits
    }

    pub fn nice(&self) -> &AtomicNice {
        &self.nice
    }

    pub fn main_thread(&self) -> Arc<Thread> {
        self.tasks.lock().main().as_thread().unwrap().clone()
    }

    // *********** Parent and child ***********

    pub fn parent(&self) -> &ParentProcess {
        &self.parent
    }

    pub fn is_init_process(&self) -> bool {
        self.parent.lock().process().upgrade().is_none()
    }

    pub(super) fn children(&self) -> &Mutex<BTreeMap<Pid, Arc<Process>>> {
        &self.children
    }

    pub fn children_wait_queue(&self) -> &WaitQueue {
        &self.children_wait_queue
    }

    // *********** Process group & Session ***********

    /// Returns the process group ID of the process.
    //
    // FIXME: If we call this method on a non-current process without holding the process table
    // lock, it may return zero if the process is reaped at the same time.
    pub fn pgid(&self) -> Pgid {
        self.process_group
            .lock()
            .upgrade()
            .map_or(0, |group| group.pgid())
    }

    /// Returns the session ID of the process.
    //
    // FIXME: If we call this method on a non-current process without holding the process table
    // lock, it may return zero if the process is reaped at the same time.
    pub fn sid(&self) -> Sid {
        self.process_group
            .lock()
            .upgrade()
            .and_then(|group| group.session())
            .map_or(0, |session| session.sid())
    }

    /// Returns the controlling terminal of the process, if any.
    pub fn terminal(&self) -> Option<Arc<dyn Terminal>> {
        self.process_group
            .lock()
            .upgrade()
            .and_then(|group| group.session())
            .and_then(|session| session.lock().terminal().cloned())
    }

    /// Moves the process to the new session.
    ///
    /// This method will create a new process group in a new session, move the process to the new
    /// session, and return the session ID (which is equal to the process ID and the process group
    /// ID).
    ///
    /// # Errors
    ///
    /// This method will return `EPERM` if an existing process group has the same identifier as the
    /// process ID. This means that the process is or was a process group leader and that the
    /// process group is still alive.
    pub fn to_new_session(self: &Arc<Self>) -> Result<Sid> {
        // Lock order: session table -> group table -> group of process
        // -> group inner -> session inner
        let mut session_table_mut = process_table::session_table_mut();
        let mut group_table_mut = process_table::group_table_mut();

        if session_table_mut.contains_key(&self.pid) {
            // FIXME: According to the Linux implementation, this check should be removed, so we'll
            // return `EPERM` due to hitting the following check. However, we need to work around a
            // gVisor bug. The upstream gVisor has fixed the issue in:
            // <https://github.com/google/gvisor/commit/582f7bf6c0ccccaeb1215a232709df38d5d409f7>.
            return Ok(self.pid);
        }
        if group_table_mut.contains_key(&self.pid) {
            return_errno_with_message!(
                Errno::EPERM,
                "a process group leader cannot be moved to a new session"
            );
        }

        let mut process_group_mut = self.process_group.lock();

        self.clear_old_group_and_session(
            &mut process_group_mut,
            &mut session_table_mut,
            &mut group_table_mut,
        );

        Ok(self.set_new_session(
            &mut process_group_mut,
            &mut session_table_mut,
            &mut group_table_mut,
        ))
    }

    pub(super) fn clear_old_group_and_session(
        &self,
        process_group_mut: &mut MutexGuard<Weak<ProcessGroup>>,
        session_table_mut: &mut MutexGuard<BTreeMap<Sid, Arc<Session>>>,
        group_table_mut: &mut MutexGuard<BTreeMap<Pgid, Arc<ProcessGroup>>>,
    ) {
        let process_group = process_group_mut.upgrade().unwrap();
        let mut process_group_inner = process_group.lock();
        let session = process_group.session().unwrap();
        let mut session_inner = session.lock();

        // Remove the process from the process group.
        process_group_inner.remove_process(&self.pid);
        if process_group_inner.is_empty() {
            group_table_mut.remove(&process_group.pgid());

            // Remove the process group from the session.
            session_inner.remove_process_group(&process_group.pgid());
            if session_inner.is_empty() {
                session_table_mut.remove(&session.sid());
            }
        }

        **process_group_mut = Weak::new();
    }

    fn set_new_session(
        self: &Arc<Self>,
        process_group_mut: &mut MutexGuard<Weak<ProcessGroup>>,
        session_table_mut: &mut MutexGuard<BTreeMap<Sid, Arc<Session>>>,
        group_table_mut: &mut MutexGuard<BTreeMap<Pgid, Arc<ProcessGroup>>>,
    ) -> Sid {
        let (session, process_group) = Session::new_pair(self.clone());
        let sid = session.sid();

        **process_group_mut = Arc::downgrade(&process_group);

        // Insert the new session and the new process group to the global table.
        session_table_mut.insert(session.sid(), session);
        group_table_mut.insert(process_group.pgid(), process_group);

        sid
    }

    /// Moves the process itself or its child process to another process group.
    ///
    /// The process to be moved is specified with the process ID `pid`; `self` is used only for
    /// permission checking purposes (see the Errors section below), which is typically
    /// `current!()` when implementing system calls.
    ///
    /// If `pgid` is equal to the process ID, a new process group with the given PGID will be
    /// created (if it does not already exist). Then, the process will be moved to the process
    /// group with the given PGID, if the process group exists and belongs to the same session as
    /// the given process.
    ///
    /// # Errors
    ///
    /// This method will return `ESRCH` in following cases:
    ///  * The process specified by `pid` does not exist;
    ///  * The process specified by `pid` is neither `self` or a child process of `self`.
    ///
    /// This method will return `EPERM` in following cases:
    ///  * The process is not in the same session as `self`;
    ///  * The process is a session leader, but the given PGID is not the process's PID/PGID;
    ///  * The process group already exists, but the group does not belong to the same session;
    ///  * The process group does not exist, but `pgid` is not equal to the process ID.
    pub fn move_process_to_group(&self, pid: Pid, pgid: Pgid) -> Result<()> {
        // Lock order: group table -> process table -> group of process
        // -> group inner -> session inner
        let group_table_mut = process_table::group_table_mut();
        let process_table_mut = process_table::process_table_mut();

        let process = process_table_mut.get(pid).ok_or(Error::with_message(
            Errno::ESRCH,
            "the process to set the PGID does not exist",
        ))?;

        let current_session = if self.pid == process.pid() {
            // There is no need to check if the session is the same in this case.
            None
        } else if self.pid == process.parent().pid() {
            // FIXME: If the child process has called `execve`, we should fail with `EACCESS`.

            // Immediately release the `self.process_group` lock to avoid deadlocks. Race
            // conditions don't matter because this is used for comparison purposes only.
            Some(
                self.process_group
                    .lock()
                    .upgrade()
                    .unwrap()
                    .session()
                    .unwrap(),
            )
        } else {
            return_errno_with_message!(
                Errno::ESRCH,
                "the process to set the PGID is neither the current process nor its child process"
            );
        };

        if let Some(new_process_group) = group_table_mut.get(&pgid).cloned() {
            process.to_existing_group(current_session, group_table_mut, new_process_group)
        } else if pgid == process.pid() {
            process.to_new_group(current_session, group_table_mut)
        } else {
            return_errno_with_message!(Errno::EPERM, "the new process group does not exist");
        }
    }

    /// Moves the process to an existing group.
    fn to_existing_group(
        self: &Arc<Self>,
        current_session: Option<Arc<Session>>,
        mut group_table_mut: MutexGuard<BTreeMap<Pgid, Arc<ProcessGroup>>>,
        new_process_group: Arc<ProcessGroup>,
    ) -> Result<()> {
        let mut process_group_mut = self.process_group.lock();
        let process_group = process_group_mut.upgrade().unwrap();

        let session = process_group.session().unwrap();
        if session.sid() == self.pid {
            return_errno_with_message!(
                Errno::EPERM,
                "a session leader cannot be moved to a new process group"
            );
        }
        if !Arc::ptr_eq(&session, &new_process_group.session().unwrap()) {
            return_errno_with_message!(
                Errno::EPERM,
                "the new process group does not belong to the same session"
            );
        }
        if current_session.is_some_and(|current| !Arc::ptr_eq(&current, &session)) {
            return_errno_with_message!(Errno::EPERM, "the process belongs to a different session");
        }

        // Lock order: group with a smaller PGID -> group with a larger PGID
        let (mut process_group_inner, mut new_group_inner) =
            match process_group.pgid().cmp(&new_process_group.pgid()) {
                core::cmp::Ordering::Less => {
                    let process_group_inner = process_group.lock();
                    let new_group_inner = new_process_group.lock();
                    (process_group_inner, new_group_inner)
                }
                core::cmp::Ordering::Greater => {
                    let new_group_inner = new_process_group.lock();
                    let process_group_inner = process_group.lock();
                    (process_group_inner, new_group_inner)
                }
                core::cmp::Ordering::Equal => return Ok(()),
            };
        let mut session_inner = session.lock();

        // Remove the process from the old process group
        process_group_inner.remove_process(&self.pid);
        if process_group_inner.is_empty() {
            group_table_mut.remove(&process_group.pgid());
            session_inner.remove_process_group(&process_group.pgid());
        }

        // Insert the process to the new process group
        new_group_inner.insert_process(self.clone());
        *process_group_mut = Arc::downgrade(&new_process_group);

        Ok(())
    }

    /// Creates a new process group and moves the process to the group.
    fn to_new_group(
        self: &Arc<Self>,
        current_session: Option<Arc<Session>>,
        mut group_table_mut: MutexGuard<BTreeMap<Pgid, Arc<ProcessGroup>>>,
    ) -> Result<()> {
        let mut process_group_mut = self.process_group.lock();

        let process_group = process_group_mut.upgrade().unwrap();
        let session = process_group.session().unwrap();

        if current_session.is_some_and(|current| !Arc::ptr_eq(&current, &session)) {
            return_errno_with_message!(Errno::EPERM, "the process belongs to a different session");
        }
        if process_group.pgid() == self.pid {
            // We'll hit this if the process is a session leader. There is no need to check below.
            return Ok(());
        }

        let mut process_group_inner = process_group.lock();
        let mut session_inner = session.lock();

        // Remove the process from the old process group
        process_group_inner.remove_process(&self.pid);
        if process_group_inner.is_empty() {
            group_table_mut.remove(&process_group.pgid());
            session_inner.remove_process_group(&process_group.pgid());
        }

        // Create a new process group and insert the process to it
        let new_process_group = ProcessGroup::new(self.clone(), Arc::downgrade(&session));
        *process_group_mut = Arc::downgrade(&new_process_group);
        group_table_mut.insert(new_process_group.pgid(), new_process_group.clone());
        session_inner.insert_process_group(new_process_group);

        Ok(())
    }

    // ************** Virtual Memory *************

    pub fn vm(&self) -> &ProcessVm {
        &self.process_vm
    }

    pub fn lock_root_vmar(&self) -> ProcessVmarGuard {
        self.process_vm.lock_root_vmar()
    }

    pub fn heap(&self) -> &ProgramBreak {
        self.process_vm.heap()
    }

    pub fn init_stack_reader(&self) -> InitStackReader {
        self.process_vm.init_stack_reader()
    }

    // ****************** Signal ******************

    pub fn sig_dispositions(&self) -> &Arc<Mutex<SigDispositions>> {
        &self.sig_dispositions
    }

    /// Enqueues a process-directed signal.
    ///
    /// This method should only be used for enqueue kernel signals and fault signals.
    ///
    /// The signal may be delivered to any one of the threads that does not currently have the
    /// signal blocked. If more than one of the threads have the signal unblocked, then this method
    /// chooses an arbitrary thread to which to deliver the signal.
    //
    // TODO: Restrict this method with the access control tool.
    pub fn enqueue_signal(&self, signal: impl Signal + Clone + 'static) {
        if self.status.is_zombie() {
            return;
        }

        let sig_dispositions = self.sig_dispositions.lock();

        // Drop the signal if it's ignored. See explanation at `enqueue_signal_locked`.
        let signum = signal.num();
        if sig_dispositions.get(signum).will_ignore(signum) {
            return;
        }

        let threads = self.tasks.lock();

        // Enqueue the signal to the first thread that does not block the signal.
        for thread in threads.as_slice() {
            let posix_thread = thread.as_posix_thread().unwrap();
            if !posix_thread.has_signal_blocked(signal.num()) {
                posix_thread.enqueue_signal_locked(Box::new(signal), sig_dispositions);
                return;
            }
        }

        // If all threads block the signal, enqueue the signal to the main thread.
        let thread = threads.main();
        let posix_thread = thread.as_posix_thread().unwrap();
        posix_thread.enqueue_signal_locked(Box::new(signal), sig_dispositions);
    }

    /// Clears the parent death signal.
    pub fn clear_parent_death_signal(&self) {
        self.parent_death_signal.clear();
    }

    /// Sets the parent death signal as `signum`.
    pub fn set_parent_death_signal(&self, sig_num: SigNum) {
        self.parent_death_signal.set(sig_num);
    }

    /// Returns the parent death signal.
    ///
    /// The parent death signal is the signal will be sent to child processes
    /// when the process exits.
    pub fn parent_death_signal(&self) -> Option<SigNum> {
        self.parent_death_signal.as_sig_num()
    }

    pub fn set_exit_signal(&self, sig_num: SigNum) {
        self.exit_signal.set(sig_num);
    }

    pub fn exit_signal(&self) -> Option<SigNum> {
        self.exit_signal.as_sig_num()
    }

    // ******************* Status ********************

    /// Returns a reference to the process status.
    pub fn status(&self) -> &ProcessStatus {
        &self.status
    }

    /// Stops the process.
    //
    // FIXME: `ptrace` is another reason that can cause a process to stop.
    // Consider extending the method signature to support `ptrace` if necessary.
    pub fn stop(&self, sig_num: SigNum) {
        if self.status.stop_status().stop(sig_num) {
            self.wake_up_parent();
        }
    }

    /// Resumes the stopped process.
    pub fn resume(&self) {
        if self.status.stop_status().resume() {
            self.wake_up_parent();

            // Note that the resume function is called by the thread which deals with SIGCONT,
            // since SIGCONT is handled by any thread in this process, we need to wake
            // up other stopped threads in the same process.
            for task in self.tasks.lock().as_slice() {
                let posix_thread = task.as_posix_thread().unwrap();
                posix_thread.wake_signalled_waker();
            }
        }
    }

    /// Returns whether the process is stopped.
    pub fn is_stopped(&self) -> bool {
        self.status.stop_status().is_stopped()
    }

    /// Gets and clears the stop status changes for the `wait` syscall.
    pub(super) fn wait_stopped_or_continued(&self, options: WaitOptions) -> Option<StopWaitStatus> {
        self.status.stop_status().wait(options)
    }

    fn wake_up_parent(&self) {
        let parent_guard = self.parent.lock();
        let parent = parent_guard.process().upgrade().unwrap();
        parent.children_wait_queue.wake_all();
    }

    // ******************* Subreaper ********************

    /// Sets the child subreaper attribute of the current process.
    pub fn set_child_subreaper(&self) {
        self.is_child_subreaper.store(true, Ordering::Release);
        let has_child_subreaper = self.has_child_subreaper.fetch_or(true, Ordering::AcqRel);
        if !has_child_subreaper {
            self.propagate_has_child_subreaper();
        }
    }

    /// Unsets the child subreaper attribute of the current process.
    pub fn unset_child_subreaper(&self) {
        self.is_child_subreaper.store(false, Ordering::Release);
    }

    /// Returns whether this process is a child subreaper.
    pub fn is_child_subreaper(&self) -> bool {
        self.is_child_subreaper.load(Ordering::Acquire)
    }

    /// Sets all descendants of the current process as having child subreaper.
    fn propagate_has_child_subreaper(&self) {
        let mut process_queue = VecDeque::new();
        let children = self.children().lock();
        for child_process in children.values() {
            if !child_process.has_child_subreaper.load(Ordering::Acquire) {
                process_queue.push_back(child_process.clone());
            }
        }

        while let Some(process) = process_queue.pop_front() {
            process.has_child_subreaper.store(true, Ordering::Release);
            let children = process.children().lock();
            for child_process in children.values() {
                if !child_process.has_child_subreaper.load(Ordering::Acquire) {
                    process_queue.push_back(child_process.clone());
                }
            }
        }
    }
}

/// Enqueues a process-directed kernel signal asynchronously.
///
/// This is the asynchronous version of [`Process::enqueue_signal`]. By asynchronous, this method
/// submits a work item and returns, so this method doesn't sleep and can be used in atomic mode.
pub fn enqueue_signal_async(process: Weak<Process>, signum: SigNum) {
    use super::signal::signals::kernel::KernelSignal;
    use crate::thread::work_queue;

    work_queue::submit_work_func(
        move || {
            if let Some(process) = process.upgrade() {
                process.enqueue_signal(KernelSignal::new(signum));
            }
        },
        work_queue::WorkPriority::High,
    );
}

/// Broadcasts a process-directed kernel signal asynchronously.
///
/// This is the asynchronous version of [`ProcessGroup::broadcast_signal`]. By asynchronous, this
/// method submits a work item and returns, so this method doesn't sleep and can be used in atomic
/// mode.
pub fn broadcast_signal_async(process_group: Weak<ProcessGroup>, signum: SigNum) {
    use super::signal::signals::kernel::KernelSignal;
    use crate::thread::work_queue;

    work_queue::submit_work_func(
        move || {
            if let Some(process_group) = process_group.upgrade() {
                process_group.broadcast_signal(KernelSignal::new(signum));
            }
        },
        work_queue::WorkPriority::High,
    );
}
