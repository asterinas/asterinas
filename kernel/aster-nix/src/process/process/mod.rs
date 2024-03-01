// SPDX-License-Identifier: MPL-2.0

use super::{
    posix_thread::PosixThreadExt,
    process_table,
    process_vm::{user_heap::UserHeap, ProcessVm},
    rlimit::ResourceLimits,
    signal::{
        constants::SIGCHLD, sig_disposition::SigDispositions, sig_mask::SigMask, signals::Signal,
        Pauser,
    },
    status::ProcessStatus,
    Credentials, TermStatus,
};
use crate::{
    device::tty::open_ntty_as_controlling_terminal,
    fs::{file_table::FileTable, fs_resolver::FsResolver, utils::FileCreationMask},
    prelude::*,
    sched::nice::Nice,
    thread::{allocate_tid, Thread},
    vm::vmar::Vmar,
};

mod builder;
mod job_control;
mod process_group;
mod session;
mod terminal;

use aster_rights::Full;
use atomic::Atomic;
pub use builder::ProcessBuilder;
pub use job_control::JobControl;
pub use process_group::ProcessGroup;
pub use session::Session;
pub use terminal::Terminal;

/// Process id.
pub type Pid = u32;
/// Process group id.
pub type Pgid = u32;
/// Session Id.
pub type Sid = u32;

pub type ExitCode = i32;

/// Process stands for a set of threads that shares the same userspace.
pub struct Process {
    // Immutable Part
    pid: Pid,

    process_vm: ProcessVm,
    /// Wait for child status changed
    children_pauser: Arc<Pauser>,

    // Mutable Part
    /// The executable path.
    executable_path: RwLock<String>,
    /// The threads
    threads: Mutex<Vec<Arc<Thread>>>,
    /// Process status
    status: Mutex<ProcessStatus>,
    /// Parent process
    pub(super) parent: Mutex<Weak<Process>>,
    /// Children processes
    children: Mutex<BTreeMap<Pid, Arc<Process>>>,
    /// Process group
    pub(super) process_group: Mutex<Weak<ProcessGroup>>,
    /// File table
    file_table: Arc<Mutex<FileTable>>,
    /// FsResolver
    fs: Arc<RwMutex<FsResolver>>,
    /// umask
    umask: Arc<RwLock<FileCreationMask>>,
    /// resource limits
    resource_limits: Mutex<ResourceLimits>,
    /// Scheduling priority nice value
    /// According to POSIX.1, the nice value is a per-process attribute,
    /// the threads in a process should share a nice value.
    nice: Atomic<Nice>,

    // Signal
    /// Sig dispositions
    sig_dispositions: Arc<Mutex<SigDispositions>>,
}

impl Process {
    #[allow(clippy::too_many_arguments)]
    fn new(
        pid: Pid,
        parent: Weak<Process>,
        threads: Vec<Arc<Thread>>,
        executable_path: String,
        process_vm: ProcessVm,
        file_table: Arc<Mutex<FileTable>>,
        fs: Arc<RwMutex<FsResolver>>,
        umask: Arc<RwLock<FileCreationMask>>,
        sig_dispositions: Arc<Mutex<SigDispositions>>,
        resource_limits: ResourceLimits,
        nice: Nice,
    ) -> Self {
        let children_pauser = {
            // SIGCHID does not interrupt pauser. Child process will
            // resume paused parent when doing exit.
            let sigmask = SigMask::from(SIGCHLD);
            Pauser::new_with_mask(sigmask)
        };

        Self {
            pid,
            threads: Mutex::new(threads),
            executable_path: RwLock::new(executable_path),
            process_vm,
            children_pauser,
            status: Mutex::new(ProcessStatus::Uninit),
            parent: Mutex::new(parent),
            children: Mutex::new(BTreeMap::new()),
            process_group: Mutex::new(Weak::new()),
            file_table,
            fs,
            umask,
            sig_dispositions,
            resource_limits: Mutex::new(resource_limits),
            nice: Atomic::new(nice),
        }
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
            let pid = allocate_tid();
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
        let threads = self.threads.lock();
        // when run the process, the process should has only one thread
        debug_assert!(threads.len() == 1);
        debug_assert!(self.is_runnable());
        let thread = threads[0].clone();
        // should not hold the lock when run thread
        drop(threads);
        thread.run();
    }

    // *********** Basic structures ***********

    pub fn pid(&self) -> Pid {
        self.pid
    }

    pub fn threads(&self) -> &Mutex<Vec<Arc<Thread>>> {
        &self.threads
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

    pub fn nice(&self) -> &Atomic<Nice> {
        &self.nice
    }

    pub fn main_thread(&self) -> Option<Arc<Thread>> {
        self.threads
            .lock()
            .iter()
            .find(|thread| thread.tid() == self.pid)
            .cloned()
    }

    // *********** Parent and child ***********
    pub fn parent(&self) -> Option<Arc<Process>> {
        self.parent.lock().upgrade()
    }

    pub fn is_init_process(&self) -> bool {
        self.parent().is_none()
    }

    pub(super) fn children(&self) -> &Mutex<BTreeMap<Pid, Arc<Process>>> {
        &self.children
    }

    pub fn has_child(&self, pid: &Pid) -> bool {
        self.children.lock().contains_key(pid)
    }

    pub fn children_pauser(&self) -> &Arc<Pauser> {
        &self.children_pauser
    }

    // *********** Process group & Session***********

    /// Returns the process group id of the process.
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
    /// * `EPERM`, if the process is a process group leader, or some existing session
    /// or process group has the same id as the process.
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
    /// * If the group already exists, the process and the group should belong to the same session.
    /// * If the group does not exist, this method creates a new group for the process and move the
    /// process to the group. The group is added to the session of the process.
    ///
    /// This method may return `EPERM` in following cases:
    /// * The process is session leader;
    /// * The group already exists, but the group does not belong to the same session as the process;
    /// * The group does not exist, but `pgid` is not equal to `pid` of the process.
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
                    "the new process group should have the same id as the process."
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

    pub fn user_heap(&self) -> &UserHeap {
        self.process_vm.user_heap()
    }

    // ************** File system ****************

    pub fn file_table(&self) -> &Arc<Mutex<FileTable>> {
        &self.file_table
    }

    pub fn fs(&self) -> &Arc<RwMutex<FsResolver>> {
        &self.fs
    }

    pub fn umask(&self) -> &Arc<RwLock<FileCreationMask>> {
        &self.umask
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
        let threads = self.threads.lock();
        for thread in threads.iter() {
            let posix_thread = thread.as_posix_thread().unwrap();
            if !posix_thread.has_signal_blocked(&signal) {
                posix_thread.enqueue_signal(Box::new(signal));
                return;
            }
        }

        // If all threads block the signal, enqueue signal to the first thread
        let thread = threads.iter().next().unwrap();
        let posix_thread = thread.as_posix_thread().unwrap();
        posix_thread.enqueue_signal(Box::new(signal));
    }

    // ******************* Status ********************

    fn set_runnable(&self) {
        self.status.lock().set_runnable();
    }

    fn is_runnable(&self) -> bool {
        self.status.lock().is_runnable()
    }

    pub fn is_zombie(&self) -> bool {
        self.status.lock().is_zombie()
    }

    pub fn set_zombie(&self, term_status: TermStatus) {
        *self.status.lock() = ProcessStatus::Zombie(term_status);
    }

    pub fn exit_code(&self) -> Option<u32> {
        match &*self.status.lock() {
            ProcessStatus::Runnable | ProcessStatus::Uninit => None,
            ProcessStatus::Zombie(term_status) => Some(term_status.as_u32()),
        }
    }
}

pub fn current() -> Arc<Process> {
    let current_thread = Thread::current();
    if let Some(posix_thread) = current_thread.as_posix_thread() {
        posix_thread.process()
    } else {
        panic!("[Internal error]The current thread does not belong to a process");
    }
}

#[cfg(ktest)]
mod test {
    use super::*;

    fn new_process(parent: Option<Arc<Process>>) -> Arc<Process> {
        crate::fs::rootfs::init_root_mount();
        let pid = allocate_tid();
        let parent = if let Some(parent) = parent {
            Arc::downgrade(&parent)
        } else {
            Weak::new()
        };
        Arc::new(Process::new(
            pid,
            parent,
            vec![],
            String::new(),
            ProcessVm::alloc(),
            Arc::new(Mutex::new(FileTable::new())),
            Arc::new(RwMutex::new(FsResolver::new())),
            Arc::new(RwLock::new(FileCreationMask::default())),
            Arc::new(Mutex::new(SigDispositions::default())),
            ResourceLimits::default(),
            Nice::default(),
        ))
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
        let process = new_process(None);
        assert!(process.process_group().is_none());
        assert!(process.session().is_none());
    }

    #[ktest]
    fn init_process_in_session() {
        let process = new_process_in_session(None);
        assert!(process.is_group_leader());
        assert!(process.is_session_leader());
        remove_session_and_group(process);
    }

    #[ktest]
    fn to_new_session() {
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
