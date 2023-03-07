use core::sync::atomic::{AtomicI32, Ordering};

use self::posix_thread::posix_thread_ext::PosixThreadExt;
use self::process_group::ProcessGroup;
use self::process_vm::user_heap::UserHeap;
use self::process_vm::UserVm;
use self::rlimit::ResourceLimits;
use self::signal::constants::SIGCHLD;
use self::signal::sig_disposition::SigDispositions;
use self::signal::sig_queues::SigQueues;
use self::signal::signals::kernel::KernelSignal;
use self::status::ProcessStatus;
use crate::fs::file_table::FileTable;
use crate::fs::fs_resolver::FsResolver;
use crate::prelude::*;
use crate::rights::Full;
use crate::thread::{thread_table, Thread};
use crate::tty::get_n_tty;
use crate::vm::vmar::Vmar;
use jinux_frame::sync::WaitQueue;
use jinux_frame::task::Task;

pub mod clone;
pub mod elf;
pub mod fifo_scheduler;
pub mod posix_thread;
pub mod process_filter;
pub mod process_group;
pub mod process_table;
pub mod process_vm;
pub mod rlimit;
pub mod signal;
pub mod status;
pub mod wait;

pub type Pid = i32;
pub type Pgid = i32;
pub type ExitCode = i32;

const INIT_PROCESS_PID: Pid = 1;

/// Process stands for a set of threads that shares the same userspace.
/// Currently, we only support one thread inside a process.
pub struct Process {
    // Immutable Part
    pid: Pid,
    elf_path: Option<CString>,
    user_vm: Option<UserVm>,
    root_vmar: Arc<Vmar<Full>>,
    /// wait for child status changed
    waiting_children: WaitQueue,
    /// wait for io events
    poll_queue: WaitQueue,

    // Mutable Part
    /// The threads
    threads: Mutex<Vec<Arc<Thread>>>,
    /// The exit code
    exit_code: AtomicI32,
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
    /// resource limits
    resource_limits: Mutex<ResourceLimits>,

    // Signal
    /// sig dispositions
    sig_dispositions: Arc<Mutex<SigDispositions>>,
    /// Process-level signal queues
    sig_queues: Mutex<SigQueues>,
}

impl Process {
    /// returns the current process
    pub fn current() -> Arc<Process> {
        let current_thread = Thread::current();
        if let Some(posix_thread) = current_thread.as_posix_thread() {
            posix_thread.process()
        } else {
            panic!("[Internal error]The current thread does not belong to a process");
        }
    }

    /// create a new process(not schedule it)
    pub fn new(
        pid: Pid,
        parent: Weak<Process>,
        threads: Vec<Arc<Thread>>,
        elf_path: Option<CString>,
        user_vm: Option<UserVm>,
        root_vmar: Arc<Vmar<Full>>,
        process_group: Weak<ProcessGroup>,
        file_table: Arc<Mutex<FileTable>>,
        fs: Arc<RwLock<FsResolver>>,
        sig_dispositions: Arc<Mutex<SigDispositions>>,
    ) -> Self {
        let children = BTreeMap::new();
        let waiting_children = WaitQueue::new();
        let poll_queue = WaitQueue::new();
        let resource_limits = ResourceLimits::default();
        Self {
            pid,
            threads: Mutex::new(threads),
            elf_path,
            user_vm,
            root_vmar,
            waiting_children,
            poll_queue,
            exit_code: AtomicI32::new(0),
            status: Mutex::new(ProcessStatus::Runnable),
            parent: Mutex::new(parent),
            children: Mutex::new(children),
            process_group: Mutex::new(process_group),
            file_table,
            fs,
            sig_dispositions,
            sig_queues: Mutex::new(SigQueues::new()),
            resource_limits: Mutex::new(resource_limits),
        }
    }

    pub fn waiting_children(&self) -> &WaitQueue {
        &self.waiting_children
    }

    pub fn poll_queue(&self) -> &WaitQueue {
        &self.poll_queue
    }

    /// init a user process and run the process
    pub fn spawn_user_process(
        filename: CString,
        elf_file_content: &'static [u8],
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Arc<Self> {
        let process = Process::create_user_process(filename, elf_file_content, argv, envp);
        // FIXME: How to determine the fg process group?
        let pgid = process.pgid();
        // FIXME: tty should be a parameter?
        let tty = get_n_tty();
        tty.set_fg(pgid);
        process.run();
        process
    }

    fn create_user_process(
        elf_path: CString,
        elf_file_content: &'static [u8],
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Arc<Self> {
        let user_process = Arc::new_cyclic(|weak_process_ref| {
            let weak_process = weak_process_ref.clone();
            let cloned_filename = Some(elf_path.clone());
            let root_vmar = Vmar::<Full>::new_root().unwrap();
<<<<<<< HEAD
            let thread = Thread::new_posix_thread_from_elf(
=======
            let fs = FsResolver::new();
            let thread = Thread::new_posix_thread_from_executable(
>>>>>>> 0255134... fix
                &root_vmar,
                elf_path,
                elf_file_content,
                weak_process,
                argv,
                envp,
            );
            let pid = thread.tid();
            // spawn process will be called in a kernel thread, so the parent process is always none.
            let parent = Weak::new();
            let user_vm = UserVm::new();
            let file_table = FileTable::new_with_stdio();
            let fs = FsResolver::new();
            let sig_dispositions = SigDispositions::new();

            let process = Process::new(
                pid,
                parent,
                vec![thread],
                cloned_filename,
                Some(user_vm),
                Arc::new(root_vmar),
                Weak::new(),
                Arc::new(Mutex::new(file_table)),
                Arc::new(RwLock::new(fs)),
                Arc::new(Mutex::new(sig_dispositions)),
            );
            process
        });
        // Set process group
        user_process.create_and_set_process_group();
        process_table::add_process(user_process.clone());
        if let Some(parent) = user_process.parent() {
            parent.add_child(user_process.clone());
        }
        user_process
    }

    /// returns the pid of the process
    pub fn pid(&self) -> Pid {
        self.pid
    }

    /// returns the process group id of the process
    pub fn pgid(&self) -> Pgid {
        if let Some(process_group) = self.process_group.lock().upgrade() {
            process_group.pgid()
        } else {
            0
        }
    }

    pub fn process_group(&self) -> &Mutex<Weak<ProcessGroup>> {
        &self.process_group
    }

    /// add a child process
    pub fn add_child(&self, child: Arc<Process>) {
        let child_pid = child.pid();
        self.children.lock().insert(child_pid, child);
    }

    pub fn set_parent(&self, parent: Weak<Process>) {
        *self.parent.lock() = parent;
    }

    /// Set process group for current process. If old process group exists,
    /// remove current process from old process group.
    pub fn set_process_group(&self, process_group: Weak<ProcessGroup>) {
        if let Some(old_process_group) = self.process_group.lock().upgrade() {
            old_process_group.remove_process(self.pid());
        }
        *self.process_group.lock() = process_group;
    }

    pub fn file_table(&self) -> &Arc<Mutex<FileTable>> {
        &self.file_table
    }

    pub fn fs(&self) -> &Arc<RwLock<FsResolver>> {
        &self.fs
    }

    /// create a new process group for the process and add it to globle table.
    /// Then set the process group for current process.
    fn create_and_set_process_group(self: &Arc<Self>) {
        let process_group = Arc::new(ProcessGroup::new(self.clone()));
        let pgid = process_group.pgid();
        self.set_process_group(Arc::downgrade(&process_group));
        process_table::add_process_group(process_group);
    }

    pub fn parent(&self) -> Option<Arc<Process>> {
        self.parent.lock().upgrade()
    }

    /// Exit thread group(the process).
    /// Set the status of the process as Zombie and set exit code.
    /// Move all children to init process.
    /// Wake up the parent wait queue if parent is waiting for self.
    pub fn exit_group(&self, exit_code: i32) {
        debug!("exit group was called");
        self.status.lock().set_zombie();
        self.exit_code.store(exit_code, Ordering::Relaxed);
        for thread in &*self.threads.lock() {
            thread.exit();
        }
        // move children to the init process
        if !self.is_init_process() {
            if let Some(init_process) = get_init_process() {
                for (_, child_process) in self.children.lock().drain_filter(|_, _| true) {
                    child_process.set_parent(Arc::downgrade(&init_process));
                    init_process.add_child(child_process);
                }
            }
        }

        if let Some(parent) = self.parent() {
            // set parent sig child
            let signal = Box::new(KernelSignal::new(SIGCHLD));
            parent.sig_queues().lock().enqueue(signal);
            // wake up parent waiting children, if any
            parent.waiting_children().wake_all();
        }
    }

    /// if the current process is init process
    pub fn is_init_process(&self) -> bool {
        self.pid == 0
    }

    /// start to run current process
    pub fn run(&self) {
        let threads = self.threads.lock();
        // when run the process, the process should has only one thread
        debug_assert!(threads.len() == 1);
        let thread = threads[0].clone();
        // should not hold the lock when run thread
        drop(threads);
        thread.run();
    }

    pub fn threads(&self) -> &Mutex<Vec<Arc<Thread>>> {
        &self.threads
    }

    /// returns the user_vm
    pub fn user_vm(&self) -> Option<&UserVm> {
        self.user_vm.as_ref()
    }

    /// returns the root vmar
    pub fn root_vmar(&self) -> &Arc<Vmar<Full>> {
        &self.root_vmar
    }

    /// returns the user heap if the process does have, otherwise None
    pub fn user_heap(&self) -> Option<&UserHeap> {
        match self.user_vm {
            None => None,
            Some(ref user_vm) => Some(user_vm.user_heap()),
        }
    }

    /// free zombie child with pid, returns the exit code of child process.
    /// remove process from process group.
    pub fn reap_zombie_child(&self, pid: Pid) -> i32 {
        let child_process = self.children.lock().remove(&pid).unwrap();
        assert!(child_process.status().lock().is_zombie());
        for thread in &*child_process.threads.lock() {
            thread_table::remove_thread(thread.tid());
        }
        process_table::remove_process(child_process.pid());
        if let Some(process_group) = child_process.process_group().lock().upgrade() {
            process_group.remove_process(child_process.pid);
        }
        child_process.exit_code().load(Ordering::SeqCst)
    }

    pub fn children(&self) -> &Mutex<BTreeMap<Pid, Arc<Process>>> {
        &self.children
    }

    pub fn exit_code(&self) -> &AtomicI32 {
        &self.exit_code
    }

    /// whether the process has child process
    pub fn has_child(&self) -> bool {
        self.children.lock().len() != 0
    }

    pub fn filename(&self) -> Option<&CString> {
        self.elf_path.as_ref()
    }

    pub fn status(&self) -> &Mutex<ProcessStatus> {
        &self.status
    }

    pub fn resource_limits(&self) -> &Mutex<ResourceLimits> {
        &self.resource_limits
    }

    pub fn sig_dispositions(&self) -> &Arc<Mutex<SigDispositions>> {
        &self.sig_dispositions
    }

    pub fn sig_queues(&self) -> &Mutex<SigQueues> {
        &self.sig_queues
    }
}

/// Get the init process
pub fn get_init_process() -> Option<Arc<Process>> {
    process_table::pid_to_process(INIT_PROCESS_PID)
}
