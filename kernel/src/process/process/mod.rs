// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use self::timer_manager::PosixTimerManager;
use super::{
    pid_namespace::{
        AncestorNsPids, PidEntryWithTasklistGuard, PidNamespace, ProcessPidEntries,
        INIT_PROCESS_PID,
    },
    posix_thread::AsPosixThread,
    process_vm::{Heap, InitStackReader, ProcessVm, ProcessVmarGuard},
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
    sched::{AtomicNice, Nice},
    thread::{AsThread, Thread},
    time::clocks::ProfClock,
};

mod current;
mod init_proc;
mod job_control;
mod process_group;
mod session;
mod terminal;
mod timer_manager;

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
pub use current::{AsCurrentProcess, CurrentProcess};
pub use init_proc::spawn_init_process;
pub use job_control::JobControl;
use ostd::{sync::WaitQueue, task::Task};
pub use process_group::ProcessGroup;
pub use session::Session;
use spin::Once;
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
    pub(super) process_group: SpinLock<Weak<ProcessGroup>>,
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

    // PID Namespace
    /// The process's IDs across all PID namespaces.
    ns_pids: AncestorNsPids,
    /// The PID namespace to which the process belongs.
    pid_namespace: Arc<PidNamespace>,
    /// The PID namespace into which newly cloned children will be placed.
    /// This field is initialized only when the process calls `unshare(CLONE_NEWPID)`.
    /// If this field is not initialized,
    /// newly cloned children will be in the same PID namespace as the process by default.
    pid_ns_for_children: Once<Arc<PidNamespace>>,
}

/// Representing a parent process by holding a weak reference to it and its PID.
///
/// This type caches the value of the PID so that it can be retrieved cheaply.
///
/// The benefit of using `ParentProcess` over `(Mutex<Weak<Process>>, AtomicPid,)` is to
/// enforce the invariant that the cached PID and the weak reference are always kept in sync.
pub struct ParentProcess {
    process: Mutex<Weak<Process>>,
    pid_ns: Arc<PidNamespace>,
    pid: AtomicPid,
}

impl ParentProcess {
    pub fn new(process: Weak<Process>, pid_ns: Arc<PidNamespace>) -> Self {
        let pid = match process.upgrade() {
            Some(process) => pid_ns.get_current_id(&process.ns_pids).unwrap_or(0),
            None => 0,
        };

        Self {
            process: Mutex::new(process),
            pid_ns,
            pid: AtomicPid::new(pid),
        }
    }

    fn pid(&self) -> Pid {
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

    /// Update both pid and weak ref.
    pub fn set_process(&mut self, new_process: &Arc<Process>) {
        let pid = self
            .this
            .pid_ns
            .get_current_id(&new_process.ns_pids)
            .unwrap_or(0);
        self.this.pid.store(pid, Ordering::Relaxed);
        *self.guard = Arc::downgrade(new_process);
    }
}

impl Process {
    /// Returns the current process.
    ///
    /// It returns `None` if:
    ///  - the function is called in the bootstrap context;
    ///  - or if the current task is not associated with a process.
    pub fn current() -> Option<CurrentProcess> {
        Some(
            Task::current()?
                .as_current_posix_thread()?
                .as_current_process(),
        )
    }

    pub(super) fn new(
        pid: Pid,
        parent: Weak<Process>,
        executable_path: String,
        process_vm: ProcessVm,

        resource_limits: ResourceLimits,
        nice: Nice,
        sig_dispositions: Arc<Mutex<SigDispositions>>,
        ns_pids: AncestorNsPids,
        pid_ns: Arc<PidNamespace>,
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
            status: ProcessStatus::default(),
            parent: ParentProcess::new(parent, pid_ns.clone()),
            children: Mutex::new(BTreeMap::new()),
            process_group: SpinLock::new(Weak::new()),
            is_child_subreaper: AtomicBool::new(false),
            has_child_subreaper: AtomicBool::new(false),
            sig_dispositions,
            parent_death_signal: AtomicSigNum::new_empty(),
            exit_signal: AtomicSigNum::new_empty(),
            resource_limits,
            nice: AtomicNice::new(nice),
            timer_manager: PosixTimerManager::new(&prof_clock, process_ref),
            prof_clock,
            ns_pids,
            pid_namespace: pid_ns,
            pid_ns_for_children: Once::new(),
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

    /// Returns the ID of parent process in the given PID namespace.
    pub fn parent_pid_in_ns(&self, pid_ns: &Arc<PidNamespace>) -> Option<Pid> {
        let parent = self.parent.lock();
        pid_ns.get_current_id(&parent.process().upgrade()?.ns_pids)
    }

    /// Returns whether the process is the init process in its PID namespace.
    pub fn is_init_process(&self) -> bool {
        self.pid == INIT_PROCESS_PID
    }

    pub(super) fn children(&self) -> &Mutex<BTreeMap<Pid, Arc<Process>>> {
        &self.children
    }

    pub fn children_wait_queue(&self) -> &WaitQueue {
        &self.children_wait_queue
    }

    // *********** Process group & Session ***********

    /// Returns the ID of the process in the given namespace.
    ///
    /// If the process is not visible in the namespace, this method will return `None`.
    pub fn pid_in_ns(&self, pid_ns: &Arc<PidNamespace>) -> Option<Pid> {
        pid_ns.get_current_id(&self.ns_pids)
    }

    /// Returns the process group ID of the process in the given namespace.
    ///
    /// If the process group is not visible in the namespace, this method will return `None`.
    //
    // FIXME: If we call this method without holding the task list
    // lock, it may return `None` if the process is reaped at the same time.
    pub fn pgid_in_ns(&self, pid_ns: &Arc<PidNamespace>) -> Option<Pgid> {
        self.process_group.lock().upgrade()?.pgid_in_ns(pid_ns)
    }

    /// Returns the session ID of the process.
    ///
    /// If the session is not visible in the namespace, this method will return `None`.
    //
    // FIXME: If we call this method without holding the task list
    // lock, it may return `None` if the process is reaped at the same time.
    pub fn sid_in_ns(&self, pid_ns: &Arc<PidNamespace>) -> Option<Sid> {
        self.process_group
            .lock()
            .upgrade()?
            .session()?
            .sid_in_ns(pid_ns)
    }

    /// Returns the session IDs across PID namespaces for the process.
    //
    // FIXME: If we call this method without holding the task list
    // lock, it may return `None` if the process is reaped at the same time.
    pub fn ns_sids(&self) -> Option<AncestorNsPids> {
        self.process_group
            .lock()
            .upgrade()?
            .session()
            .map(|session| session.ns_sids().clone())
    }

    /// Returns the controlling terminal of the process, if any.
    pub fn terminal(&self) -> Option<Arc<dyn Terminal>> {
        self.process_group
            .lock()
            .upgrade()
            .and_then(|group| group.session())
            .and_then(|session| session.lock().terminal().cloned())
    }

    pub(super) fn clear_old_group_and_session(
        self: &Arc<Self>,
        process_group_mut: &mut Weak<ProcessGroup>,
        process_pid_entries: &mut ProcessPidEntries,
    ) {
        let process_group = process_group_mut.upgrade().unwrap();
        let mut process_group_inner = process_group.lock();
        let session = process_group.session().unwrap();
        let mut session_inner = session.lock();

        // Remove the process from the process group.
        process_group_inner.remove_process(self);
        if process_group_inner.is_empty() {
            process_pid_entries.detach_process_group();

            // Remove the process group from the session.
            session_inner.remove_process_group(&process_group);
            if session_inner.is_empty() {
                process_pid_entries.detach_session();
            }
        }

        *process_group_mut = Weak::new();
    }

    fn set_new_session(
        self: &Arc<Self>,
        process_group_mut: &mut Weak<ProcessGroup>,
        pid_entry_guard: &mut PidEntryWithTasklistGuard,
    ) -> Sid {
        let (session, process_group) = Session::new_pair(self.clone());
        let sid = session.sid_in_ns(&self.pid_namespace).unwrap();

        *process_group_mut = Arc::downgrade(&process_group);

        // Insert the new session and the new process group to the global table.
        pid_entry_guard.attach_session(session);
        pid_entry_guard.attach_process_group(process_group);

        sid
    }

    /// Moves the process to an existing group.
    fn to_existing_group(
        self: &Arc<Self>,
        current_session: Option<Arc<Session>>,
        process_group_mut: &mut Weak<ProcessGroup>,
        process_pid_entries: &mut ProcessPidEntries,
        new_process_group: Arc<ProcessGroup>,
    ) -> Result<()> {
        let process_group = process_group_mut.upgrade().unwrap();

        let session = process_group.session().unwrap();
        if let Some(sid) = session.sid_in_ns(&self.pid_namespace)
            && sid == self.pid
        {
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
            match process_group.ns_pgids().cmp(&new_process_group.ns_pgids()) {
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
        process_group_inner.remove_process(&self);
        if process_group_inner.is_empty() {
            process_pid_entries.detach_process_group();
            session_inner.remove_process_group(&process_group);
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
        process_group_mut: &mut Weak<ProcessGroup>,
        process_pid_entries: &mut ProcessPidEntries,
    ) -> Result<()> {
        let process_group = process_group_mut.upgrade().unwrap();
        let session = process_group.session().unwrap();

        if current_session.is_some_and(|current| !Arc::ptr_eq(&current, &session)) {
            return_errno_with_message!(Errno::EPERM, "the process belongs to a different session");
        }
        if let Some(pgid) = process_group.pgid_in_ns(&self.pid_namespace)
            && pgid == self.pid
        {
            // We'll hit this if the process is a session leader. There is no need to check below.
            return Ok(());
        }

        let mut process_group_inner = process_group.lock();
        let mut session_inner = session.lock();

        // Remove the process from the old process group
        process_group_inner.remove_process(self);
        if process_group_inner.is_empty() {
            process_pid_entries.detach_process_group();
            session_inner.remove_process_group(&process_group);
        }

        // Create a new process group and insert the process to it
        let new_process_group = ProcessGroup::new(self.clone(), Arc::downgrade(&session));
        *process_group_mut = Arc::downgrade(&new_process_group);
        process_pid_entries
            .process_entry_guard()
            .attach_process_group(new_process_group.clone());
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

    pub fn heap(&self) -> &Heap {
        self.process_vm.heap()
    }

    pub fn init_stack_reader(&self) -> InitStackReader {
        self.process_vm.init_stack_reader()
    }

    // ****************** Signal ******************

    pub fn sig_dispositions(&self) -> &Arc<Mutex<SigDispositions>> {
        &self.sig_dispositions
    }

    /// Enqueues a process-directed signal. This method should only be used for enqueue kernel
    /// signal and fault signal.
    ///
    /// The signal may be delivered to any one of the threads that does not currently have the
    /// signal blocked.  If more than one of the threads has the signal unblocked, then this method
    /// chooses an arbitrary thread to which to deliver the signal.
    ///
    /// TODO: restrict these method with access control tool.
    pub fn enqueue_signal(&self, signal: impl Signal + Clone + 'static) {
        if self.status.is_zombie() {
            return;
        }

        // TODO: check that the signal is not user signal

        // Enqueue signal to the first thread that does not block the signal
        let threads = self.tasks.lock();
        for thread in threads.as_slice() {
            let posix_thread = thread.as_posix_thread().unwrap();
            if !posix_thread.has_signal_blocked(signal.num()) {
                posix_thread.enqueue_signal(Box::new(signal));
                return;
            }
        }

        // If all threads block the signal, enqueue signal to the main thread
        let thread = threads.main();
        let posix_thread = thread.as_posix_thread().unwrap();
        posix_thread.enqueue_signal(Box::new(signal));
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

    // ******************* PID Namespace ********************

    /// Returns the process's IDs across all PID namespaces.
    pub fn ns_pids(&self) -> &AncestorNsPids {
        &self.ns_pids
    }

    /// Returns the PID namespace of the process.
    pub fn pid_namespace(&self) -> &Arc<PidNamespace> {
        &self.pid_namespace
    }

    /// Returns the PID namespace used for creating child processes.
    pub fn pid_ns_for_children(&self) -> &Once<Arc<PidNamespace>> {
        &self.pid_ns_for_children
    }
}
