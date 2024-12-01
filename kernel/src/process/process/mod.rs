// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use self::timer_manager::PosixTimerManager;
use super::{
    posix_thread::{allocate_posix_tid, AsPosixThread},
    process_table,
    process_vm::{Heap, InitStackReader, ProcessVm},
    rlimit::ResourceLimits,
    signal::{
        sig_disposition::SigDispositions,
        sig_num::{AtomicSigNum, SigNum},
        signals::Signal,
    },
    status::ProcessStatus,
    Credentials, TermStatus,
};
use crate::{
    device::tty::open_ntty_as_controlling_terminal,
    prelude::*,
    sched::priority::{AtomicNice, Nice},
    thread::{AsThread, Thread},
    time::clocks::ProfClock,
    vm::vmar::Vmar,
};

mod builder;
mod job_control;
mod process_group;
mod session;
mod terminal;
mod timer_manager;

use aster_rights::Full;
use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
pub use builder::ProcessBuilder;
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

    // Mutable Part
    /// The executable path.
    executable_path: RwLock<String>,
    /// The threads
    tasks: Mutex<Vec<Arc<Task>>>,
    /// Process status
    status: ProcessStatus,
    /// Parent process
    pub(super) parent: ParentProcess,
    /// Children processes
    children: Mutex<BTreeMap<Pid, Arc<Process>>>,
    /// Process group
    pub(super) process_group: Mutex<Weak<ProcessGroup>>,
    /// resource limits
    resource_limits: Mutex<ResourceLimits>,
    /// Scheduling priority nice value
    /// According to POSIX.1, the nice value is a per-process attribute,
    /// the threads in a process should share a nice value.
    nice: AtomicNice,

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

    #[allow(clippy::too_many_arguments)]
    fn new(
        pid: Pid,
        parent: Weak<Process>,
        tasks: Vec<Arc<Task>>,
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
            tasks: Mutex::new(tasks),
            executable_path: RwLock::new(executable_path),
            process_vm,
            children_wait_queue,
            status: ProcessStatus::new_uninit(),
            parent: ParentProcess::new(parent),
            children: Mutex::new(BTreeMap::new()),
            process_group: Mutex::new(Weak::new()),
            sig_dispositions,
            parent_death_signal: AtomicSigNum::new_empty(),
            exit_signal: AtomicSigNum::new_empty(),
            resource_limits: Mutex::new(resource_limits),
            nice: AtomicNice::new(nice),
            timer_manager: PosixTimerManager::new(&prof_clock, process_ref),
            prof_clock,
        })
    }

    /// init a user process and run the process
    pub fn spawn_user_process(
        executable_path: &str,
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Result<Arc<Self>> {
        // spawn user process should give an absolute path
        debug_assert!(executable_path.starts_with('/'));
        let process = Process::create_user_process(executable_path, argv, envp)?;

        open_ntty_as_controlling_terminal(&process)?;

        process.run();
        Ok(process)
    }

    fn create_user_process(
        executable_path: &str,
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Result<Arc<Self>> {
        let process_builder = {
            let pid = allocate_posix_tid();
            let parent = Weak::new();

            let credentials = Credentials::new_root();

            let mut builder = ProcessBuilder::new(pid, executable_path, parent);
            builder.argv(argv).envp(envp).credentials(credentials);
            builder
        };

        let process = process_builder.build()?;

        // Lock order: session table -> group table -> process table -> group of process
        // -> group inner -> session inner
        let mut session_table_mut = process_table::session_table_mut();
        let mut group_table_mut = process_table::group_table_mut();
        let mut process_table_mut = process_table::process_table_mut();

        // Creates new group
        let group = ProcessGroup::new(process.clone());
        *process.process_group.lock() = Arc::downgrade(&group);
        group_table_mut.insert(group.pgid(), group.clone());

        // Creates new session
        let session = Session::new(group.clone());
        group.inner.lock().session = Arc::downgrade(&session);
        session.inner.lock().leader = Some(process.clone());
        session_table_mut.insert(session.sid(), session);

        process_table_mut.insert(process.pid(), process.clone());
        Ok(process)
    }

    /// start to run current process
    pub fn run(&self) {
        let tasks = self.tasks.lock();
        // when run the process, the process should has only one thread
        debug_assert!(tasks.len() == 1);
        debug_assert!(self.is_runnable());
        let task = tasks[0].clone();
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

    pub fn tasks(&self) -> &Mutex<Vec<Arc<Task>>> {
        &self.tasks
    }

    pub fn executable_path(&self) -> String {
        self.executable_path.read().clone()
    }

    pub fn set_executable_path(&self, executable_path: String) {
        *self.executable_path.write() = executable_path;
    }

    pub fn resource_limits(&self) -> &Mutex<ResourceLimits> {
        &self.resource_limits
    }

    pub fn nice(&self) -> &AtomicNice {
        &self.nice
    }

    pub fn main_thread(&self) -> Option<Arc<Thread>> {
        self.tasks
            .lock()
            .iter()
            .find_map(|task| {
                let thread = task.as_thread().unwrap();
                (thread.as_posix_thread().unwrap().tid() == self.pid).then_some(thread)
            })
            .cloned()
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

    pub fn has_child(&self, pid: &Pid) -> bool {
        self.children.lock().contains_key(pid)
    }

    pub fn children_wait_queue(&self) -> &WaitQueue {
        &self.children_wait_queue
    }

    // *********** Process group & Session***********

    /// Returns the process group ID of the process.
    pub fn pgid(&self) -> Pgid {
        if let Some(process_group) = self.process_group.lock().upgrade() {
            process_group.pgid()
        } else {
            0
        }
    }

    /// Returns the process group which the process belongs to.
    pub fn process_group(&self) -> Option<Arc<ProcessGroup>> {
        self.process_group.lock().upgrade()
    }

    /// Returns whether `self` is the leader of process group.
    fn is_group_leader(self: &Arc<Self>) -> bool {
        let Some(process_group) = self.process_group() else {
            return false;
        };

        let Some(leader) = process_group.leader() else {
            return false;
        };

        Arc::ptr_eq(self, &leader)
    }

    /// Returns the session which the process belongs to.
    pub fn session(&self) -> Option<Arc<Session>> {
        let process_group = self.process_group()?;
        process_group.session()
    }

    /// Returns whether the process is session leader.
    pub fn is_session_leader(self: &Arc<Self>) -> bool {
        let session = self.session().unwrap();

        let Some(leading_process) = session.leader() else {
            return false;
        };

        Arc::ptr_eq(self, &leading_process)
    }

    /// Moves the process to the new session.
    ///
    /// If the process is already session leader, this method does nothing.
    ///
    /// Otherwise, this method creates a new process group in a new session
    /// and moves the process to the session, returning the new session.
    ///
    /// This method may return the following errors:
    ///  * `EPERM`, if the process is a process group leader, or some existing session
    ///    or process group has the same ID as the process.
    pub fn to_new_session(self: &Arc<Self>) -> Result<Arc<Session>> {
        if self.is_session_leader() {
            return Ok(self.session().unwrap());
        }

        if self.is_group_leader() {
            return_errno_with_message!(
                Errno::EPERM,
                "process group leader cannot be moved to new session."
            );
        }

        let session = self.session().unwrap();

        // Lock order: session table -> group table -> group of process -> group inner -> session inner
        let mut session_table_mut = process_table::session_table_mut();
        let mut group_table_mut = process_table::group_table_mut();
        let mut self_group_mut = self.process_group.lock();

        if session_table_mut.contains_key(&self.pid) {
            return_errno_with_message!(Errno::EPERM, "cannot create new session");
        }

        if group_table_mut.contains_key(&self.pid) {
            return_errno_with_message!(Errno::EPERM, "cannot create process group");
        }

        // Removes the process from old group
        if let Some(old_group) = self_group_mut.upgrade() {
            let mut group_inner = old_group.inner.lock();
            let mut session_inner = session.inner.lock();
            group_inner.remove_process(&self.pid);
            *self_group_mut = Weak::new();

            if group_inner.is_empty() {
                group_table_mut.remove(&old_group.pgid());
                debug_assert!(session_inner.process_groups.contains_key(&old_group.pgid()));
                session_inner.process_groups.remove(&old_group.pgid());

                if session_inner.is_empty() {
                    session_table_mut.remove(&session.sid());
                }
            }
        }

        // Creates a new process group
        let new_group = ProcessGroup::new(self.clone());
        *self_group_mut = Arc::downgrade(&new_group);
        group_table_mut.insert(new_group.pgid(), new_group.clone());

        // Creates a new session
        let new_session = Session::new(new_group.clone());
        let mut new_group_inner = new_group.inner.lock();
        new_group_inner.session = Arc::downgrade(&new_session);
        new_session.inner.lock().leader = Some(self.clone());
        session_table_mut.insert(new_session.sid(), new_session.clone());

        // Removes the process from session.
        let mut session_inner = session.inner.lock();
        session_inner.remove_process(self);

        Ok(new_session)
    }

    /// Moves the process to other process group.
    ///
    ///  * If the group already exists, the process and the group should belong to the same session.
    ///  * If the group does not exist, this method creates a new group for the process and move the
    ///    process to the group. The group is added to the session of the process.
    ///
    /// This method may return `EPERM` in following cases:
    ///  * The process is session leader;
    ///  * The group already exists, but the group does not belong to the same session as the process;
    ///  * The group does not exist, but `pgid` is not equal to `pid` of the process.
    pub fn to_other_group(self: &Arc<Self>, pgid: Pgid) -> Result<()> {
        // if the process already belongs to the process group
        if self.pgid() == pgid {
            return Ok(());
        }

        if self.is_session_leader() {
            return_errno_with_message!(Errno::EPERM, "the process cannot be a session leader");
        }

        if let Some(process_group) = process_table::get_process_group(&pgid) {
            let session = self.session().unwrap();
            if !session.contains_process_group(&process_group) {
                return_errno_with_message!(
                    Errno::EPERM,
                    "the group and process does not belong to same session"
                );
            }
            self.to_specified_group(&process_group)?;
        } else {
            if pgid != self.pid() {
                return_errno_with_message!(
                    Errno::EPERM,
                    "the new process group should have the same ID as the process."
                );
            }

            self.to_new_group()?;
        }

        Ok(())
    }

    /// Creates a new process group and moves the process to the group.
    ///
    /// The new group will be added to the same session as the process.
    fn to_new_group(self: &Arc<Self>) -> Result<()> {
        let session = self.session().unwrap();
        // Lock order: group table -> group of process -> group inner -> session inner
        let mut group_table_mut = process_table::group_table_mut();
        let mut self_group_mut = self.process_group.lock();

        // Removes the process from old group
        if let Some(old_group) = self_group_mut.upgrade() {
            let mut group_inner = old_group.inner.lock();
            let mut session_inner = session.inner.lock();
            group_inner.remove_process(&self.pid);
            *self_group_mut = Weak::new();

            if group_inner.is_empty() {
                group_table_mut.remove(&old_group.pgid());
                debug_assert!(session_inner.process_groups.contains_key(&old_group.pgid()));
                // The old session won't be empty, since we will add a new group to the session.
                session_inner.process_groups.remove(&old_group.pgid());
            }
        }

        // Creates a new process group. Adds the new group to group table and session.
        let new_group = ProcessGroup::new(self.clone());

        let mut new_group_inner = new_group.inner.lock();
        let mut session_inner = session.inner.lock();

        *self_group_mut = Arc::downgrade(&new_group);

        group_table_mut.insert(new_group.pgid(), new_group.clone());

        new_group_inner.session = Arc::downgrade(&session);
        session_inner
            .process_groups
            .insert(new_group.pgid(), new_group.clone());

        Ok(())
    }

    /// Moves the process to a specified group.
    ///
    /// The caller needs to ensure that the process and the group belongs to the same session.
    fn to_specified_group(self: &Arc<Process>, group: &Arc<ProcessGroup>) -> Result<()> {
        // Lock order: group table -> group of process -> group inner (small pgid -> big pgid)
        let mut group_table_mut = process_table::group_table_mut();
        let mut self_group_mut = self.process_group.lock();

        // Removes the process from old group
        let mut group_inner = if let Some(old_group) = self_group_mut.upgrade() {
            // Lock order: group with smaller pgid first
            let (mut old_group_inner, group_inner) = match old_group.pgid().cmp(&group.pgid()) {
                core::cmp::Ordering::Equal => return Ok(()),
                core::cmp::Ordering::Less => (old_group.inner.lock(), group.inner.lock()),
                core::cmp::Ordering::Greater => {
                    let group_inner = group.inner.lock();
                    let old_group_inner = old_group.inner.lock();
                    (old_group_inner, group_inner)
                }
            };
            old_group_inner.remove_process(&self.pid);
            *self_group_mut = Weak::new();

            if old_group_inner.is_empty() {
                group_table_mut.remove(&old_group.pgid());
            }

            group_inner
        } else {
            group.inner.lock()
        };

        // Adds the process to the specified group
        group_inner.processes.insert(self.pid, self.clone());
        *self_group_mut = Arc::downgrade(group);

        Ok(())
    }

    // ************** Virtual Memory *************

    pub fn vm(&self) -> &ProcessVm {
        &self.process_vm
    }

    pub fn root_vmar(&self) -> &Vmar<Full> {
        self.process_vm.root_vmar()
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
        if self.is_zombie() {
            return;
        }

        // TODO: check that the signal is not user signal

        // Enqueue signal to the first thread that does not block the signal
        let threads = self.tasks.lock();
        for thread in threads.iter() {
            let posix_thread = thread.as_posix_thread().unwrap();
            if !posix_thread.has_signal_blocked(signal.num()) {
                posix_thread.enqueue_signal(Box::new(signal));
                return;
            }
        }

        // If all threads block the signal, enqueue signal to the first thread
        let thread = threads.iter().next().unwrap();
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

    fn set_runnable(&self) {
        self.status.set_runnable();
    }

    fn is_runnable(&self) -> bool {
        self.status.is_runnable()
    }

    pub fn is_zombie(&self) -> bool {
        self.status.is_zombie()
    }

    pub fn set_zombie(&self, term_status: TermStatus) {
        self.status.set_zombie(term_status);
    }

    pub fn exit_code(&self) -> ExitCode {
        self.status.exit_code()
    }
}

#[cfg(ktest)]
mod test {

    use ostd::prelude::*;

    use super::*;

    fn new_process(parent: Option<Arc<Process>>) -> Arc<Process> {
        crate::util::random::init();
        crate::fs::rootfs::init_root_mount();
        let pid = allocate_posix_tid();
        let parent = if let Some(parent) = parent {
            Arc::downgrade(&parent)
        } else {
            Weak::new()
        };
        Process::new(
            pid,
            parent,
            vec![],
            String::new(),
            ProcessVm::alloc(),
            ResourceLimits::default(),
            Nice::default(),
            Arc::new(Mutex::new(SigDispositions::default())),
        )
    }

    fn new_process_in_session(parent: Option<Arc<Process>>) -> Arc<Process> {
        // Lock order: session table -> group table -> group of process -> group inner
        // -> session inner
        let mut session_table_mut = process_table::session_table_mut();
        let mut group_table_mut = process_table::group_table_mut();

        let process = new_process(parent);
        // Creates new group
        let group = ProcessGroup::new(process.clone());
        *process.process_group.lock() = Arc::downgrade(&group);

        // Creates new session
        let sess = Session::new(group.clone());
        group.inner.lock().session = Arc::downgrade(&sess);
        sess.inner.lock().leader = Some(process.clone());

        group_table_mut.insert(group.pgid(), group);
        session_table_mut.insert(sess.sid(), sess);

        process
    }

    fn remove_session_and_group(process: Arc<Process>) {
        // Lock order: session table -> group table
        let mut session_table_mut = process_table::session_table_mut();
        let mut group_table_mut = process_table::group_table_mut();
        if let Some(sess) = process.session() {
            session_table_mut.remove(&sess.sid());
        }

        if let Some(group) = process.process_group() {
            group_table_mut.remove(&group.pgid());
        }
    }

    #[ktest]
    fn init_process() {
        crate::time::clocks::init_for_ktest();
        let process = new_process(None);
        assert!(process.process_group().is_none());
        assert!(process.session().is_none());
    }

    #[ktest]
    fn init_process_in_session() {
        crate::time::clocks::init_for_ktest();
        let process = new_process_in_session(None);
        assert!(process.is_group_leader());
        assert!(process.is_session_leader());
        remove_session_and_group(process);
    }

    #[ktest]
    fn to_new_session() {
        crate::time::clocks::init_for_ktest();
        let process = new_process_in_session(None);
        let sess = process.session().unwrap();
        sess.inner.lock().leader = None;

        assert!(!process.is_session_leader());
        assert!(process
            .to_new_session()
            .is_err_and(|e| e.error() == Errno::EPERM));

        let group = process.process_group().unwrap();
        group.inner.lock().leader = None;
        assert!(!process.is_group_leader());

        assert!(process
            .to_new_session()
            .is_err_and(|e| e.error() == Errno::EPERM));
    }
}
