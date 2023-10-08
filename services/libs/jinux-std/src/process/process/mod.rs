mod builder;

use super::posix_thread::PosixThreadExt;
use super::process_group::ProcessGroup;
use super::process_vm::user_heap::UserHeap;
use super::process_vm::ProcessVm;
use super::rlimit::ResourceLimits;
use super::signal::constants::SIGCHLD;
use super::signal::sig_disposition::SigDispositions;
use super::signal::sig_mask::SigMask;
use super::signal::sig_queues::SigQueues;
use super::signal::signals::Signal;
use super::signal::{Pauser, SigEvents, SigEventsFilter};
use super::status::ProcessStatus;
use super::{process_table, TermStatus};
use crate::device::tty::get_n_tty;
use crate::events::Observer;
use crate::fs::file_table::FileTable;
use crate::fs::fs_resolver::FsResolver;
use crate::fs::utils::FileCreationMask;
use crate::prelude::*;
use crate::thread::{allocate_tid, Thread};
use crate::vm::vmar::Vmar;
use jinux_rights::Full;

pub use builder::ProcessBuilder;

pub type Pid = u32;
pub type Pgid = u32;
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
    parent: Mutex<Weak<Process>>,
    /// Children processes
    children: Mutex<BTreeMap<Pid, Arc<Process>>>,
    /// Process group
    process_group: Mutex<Weak<ProcessGroup>>,
    /// File table
    file_table: Arc<Mutex<FileTable>>,
    /// FsResolver
    fs: Arc<RwLock<FsResolver>>,
    /// umask
    umask: Arc<RwLock<FileCreationMask>>,
    /// resource limits
    resource_limits: Mutex<ResourceLimits>,

    // Signal
    /// sig dispositions
    sig_dispositions: Arc<Mutex<SigDispositions>>,
    /// Process-level signal queues
    sig_queues: Mutex<SigQueues>,
}

impl Process {
    #[allow(clippy::too_many_arguments)]
    fn new(
        pid: Pid,
        parent: Weak<Process>,
        threads: Vec<Arc<Thread>>,
        executable_path: String,
        process_vm: ProcessVm,
        process_group: Weak<ProcessGroup>,
        file_table: Arc<Mutex<FileTable>>,
        fs: Arc<RwLock<FsResolver>>,
        umask: Arc<RwLock<FileCreationMask>>,
        sig_dispositions: Arc<Mutex<SigDispositions>>,
        resource_limits: ResourceLimits,
    ) -> Self {
        let children_pauser = {
            let mut sigset = SigMask::new_full();
            // SIGCHID does not interrupt pauser. Child process will
            // resume paused parent when doing exit.
            sigset.remove_signal(SIGCHLD);
            Pauser::new_with_sigset(sigset)
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
            process_group: Mutex::new(process_group),
            file_table,
            fs,
            umask,
            sig_dispositions,
            sig_queues: Mutex::new(SigQueues::new()),
            resource_limits: Mutex::new(resource_limits),
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
        // FIXME: How to determine the fg process group?
        let process_group = Weak::clone(&process.process_group.lock());
        // FIXME: tty should be a parameter?
        let tty = get_n_tty();
        tty.set_fg(process_group);
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
            let mut builder = ProcessBuilder::new(pid, executable_path, parent);
            builder.argv(argv).envp(envp);
            builder
        };

        let process = process_builder.build()?;
        process_table::add_process(process.clone());
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

    // *********** Parent and child ***********

    pub fn add_child(&self, child: Arc<Process>) {
        let child_pid = child.pid();
        self.children.lock().insert(child_pid, child);
    }

    pub fn set_parent(&self, parent: Weak<Process>) {
        *self.parent.lock() = parent;
    }

    pub fn parent(&self) -> Option<Arc<Process>> {
        self.parent.lock().upgrade()
    }

    pub fn children(&self) -> &Mutex<BTreeMap<Pid, Arc<Process>>> {
        &self.children
    }

    pub fn children_pauser(&self) -> &Arc<Pauser> {
        &self.children_pauser
    }

    // *********** Process group ***********

    pub fn pgid(&self) -> Pgid {
        if let Some(process_group) = self.process_group.lock().upgrade() {
            process_group.pgid()
        } else {
            0
        }
    }

    /// Set process group for current process. If old process group exists,
    /// remove current process from old process group.
    pub fn set_process_group(&self, process_group: Weak<ProcessGroup>) {
        if let Some(old_process_group) = self.process_group() {
            old_process_group.remove_process(self.pid());
        }
        *self.process_group.lock() = process_group;
    }

    pub fn process_group(&self) -> Option<Arc<ProcessGroup>> {
        self.process_group.lock().upgrade()
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

    pub fn fs(&self) -> &Arc<RwLock<FsResolver>> {
        &self.fs
    }

    pub fn umask(&self) -> &Arc<RwLock<FileCreationMask>> {
        &self.umask
    }

    // ****************** Signal ******************

    pub fn sig_dispositions(&self) -> &Arc<Mutex<SigDispositions>> {
        &self.sig_dispositions
    }

    pub fn has_pending_signal(&self) -> bool {
        !self.sig_queues.lock().is_empty()
    }

    pub fn enqueue_signal(&self, signal: Box<dyn Signal>) {
        if !self.is_zombie() {
            self.sig_queues.lock().enqueue(signal);
        }
    }

    pub fn dequeue_signal(&self, mask: &SigMask) -> Option<Box<dyn Signal>> {
        self.sig_queues.lock().dequeue(mask)
    }

    pub fn register_sigqueue_observer(
        &self,
        observer: Weak<dyn Observer<SigEvents>>,
        filter: SigEventsFilter,
    ) {
        self.sig_queues.lock().register_observer(observer, filter);
    }

    pub fn unregiser_sigqueue_observer(&self, observer: &Weak<dyn Observer<SigEvents>>) {
        self.sig_queues.lock().unregister_observer(observer);
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
