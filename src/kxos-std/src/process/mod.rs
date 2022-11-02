use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

use crate::prelude::*;
use kxos_frame::sync::WaitQueue;
use kxos_frame::{task::Task, user::UserSpace, vm::VmSpace};

use self::process_filter::ProcessFilter;
use self::process_group::ProcessGroup;
use self::process_vm::mmap_area::MmapArea;
use self::process_vm::user_heap::UserHeap;
use self::process_vm::UserVm;
use self::signal::constants::SIGCHLD;
use self::signal::sig_disposition::SigDispositions;
use self::signal::sig_mask::SigMask;
use self::signal::sig_queues::SigQueues;
use self::signal::signals::kernel::KernelSignal;
use self::status::ProcessStatus;
use self::task::create_user_task_from_elf;

pub mod clone;
pub mod elf;
pub mod exception;
pub mod fifo_scheduler;
pub mod process_filter;
pub mod process_group;
pub mod process_vm;
pub mod signal;
pub mod status;
pub mod table;
pub mod task;
pub mod wait;

static PID_ALLOCATOR: AtomicUsize = AtomicUsize::new(0);

pub type Pid = usize;
pub type Pgid = usize;
pub type ExitCode = i32;

/// Process stands for a set of tasks that shares the same userspace.
/// Currently, we only support one task inside a process.
pub struct Process {
    // Immutable Part
    pid: Pid,
    task: Arc<Task>,
    filename: Option<CString>,
    user_space: Option<Arc<UserSpace>>,
    user_vm: Option<UserVm>,
    waiting_children: WaitQueue<ProcessFilter>,

    // Mutable Part
    /// The exit code
    exit_code: AtomicI32,
    /// Process status
    status: Mutex<ProcessStatus>,
    /// Parent process
    parent: Mutex<Option<Weak<Process>>>,
    /// Children processes
    children: Mutex<BTreeMap<usize, Arc<Process>>>,
    /// Process group
    process_group: Mutex<Option<Weak<ProcessGroup>>>,

    // Signal
    sig_dispositions: Mutex<SigDispositions>,
    sig_queues: Mutex<SigQueues>,
    /// Process-level sigmask
    sig_mask: Mutex<SigMask>,
}

impl Process {
    /// returns the current process
    pub fn current() -> Arc<Process> {
        let task = Task::current();
        let process = task
            .data()
            .downcast_ref::<Weak<Process>>()
            .expect("[Internal Error] task data should points to weak<process>");
        process
            .upgrade()
            .expect("[Internal Error] current process cannot be None")
    }

    /// create a new process(not schedule it)
    pub fn new(
        pid: Pid,
        task: Arc<Task>,
        exec_filename: Option<CString>,
        user_vm: Option<UserVm>,
        user_space: Option<Arc<UserSpace>>,
        process_group: Option<Weak<ProcessGroup>>,
        sig_dispositions: SigDispositions,
        sig_queues: SigQueues,
        sig_mask: SigMask,
    ) -> Self {
        let parent = if pid == 0 {
            debug!("Init process does not has parent");
            None
        } else {
            debug!("All process except init should have parent");
            let current_process = Process::current();
            Some(Arc::downgrade(&current_process))
        };
        let children = BTreeMap::new();
        let waiting_children = WaitQueue::new();
        Self {
            pid,
            task,
            filename: exec_filename,
            user_space,
            user_vm,
            waiting_children,
            exit_code: AtomicI32::new(0),
            status: Mutex::new(ProcessStatus::Runnable),
            parent: Mutex::new(parent),
            children: Mutex::new(children),
            process_group: Mutex::new(process_group),
            sig_dispositions: Mutex::new(sig_dispositions),
            sig_queues: Mutex::new(sig_queues),
            sig_mask: Mutex::new(sig_mask),
        }
    }

    pub fn waiting_children(&self) -> &WaitQueue<ProcessFilter> {
        &self.waiting_children
    }

    /// init a user process and send the process to scheduler
    pub fn spawn_user_process(filename: CString, elf_file_content: &'static [u8]) -> Arc<Self> {
        let process = Process::create_user_process(filename, elf_file_content);
        process.send_to_scheduler();
        process
    }

    /// init a kernel process and send the process to scheduler
    pub fn spawn_kernel_process<F>(task_fn: F) -> Arc<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let process = Process::create_kernel_process(task_fn);
        process.send_to_scheduler();
        process
    }

    fn create_user_process(filename: CString, elf_file_content: &'static [u8]) -> Arc<Self> {
        let pid = new_pid();

        let user_process = Arc::new_cyclic(|weak_process_ref| {
            let weak_process = weak_process_ref.clone();
            let cloned_filename = Some(filename.clone());
            let task = create_user_task_from_elf(filename, elf_file_content, weak_process);
            let user_space = task.user_space().map(|user_space| user_space.clone());
            let user_vm = UserVm::new();
            let sig_dispositions = SigDispositions::new();
            let sig_queues = SigQueues::new();
            let sig_mask = SigMask::new_empty();
            Process::new(
                pid,
                task,
                cloned_filename,
                Some(user_vm),
                user_space,
                None,
                sig_dispositions,
                sig_queues,
                sig_mask,
            )
        });
        // Set process group
        user_process.create_and_set_process_group();
        table::add_process(pid, user_process.clone());
        user_process
    }

    fn create_kernel_process<F>(task_fn: F) -> Arc<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let pid = new_pid();
        let kernel_process = Arc::new_cyclic(|weak_process_ref| {
            let weak_process = weak_process_ref.clone();
            let task = Task::new(task_fn, weak_process, None).expect("spawn kernel task failed");
            let sig_dispositions = SigDispositions::new();
            let sig_queues = SigQueues::new();
            let sig_mask = SigMask::new_empty();
            Process::new(
                pid,
                task,
                None,
                None,
                None,
                None,
                sig_dispositions,
                sig_queues,
                sig_mask,
            )
        });
        kernel_process.create_and_set_process_group();
        table::add_process(pid, kernel_process.clone());
        kernel_process
    }

    /// returns the pid of the process
    pub fn pid(&self) -> Pid {
        self.pid
    }

    /// returns the process group id of the process
    pub fn pgid(&self) -> Pgid {
        if let Some(process_group) = self
            .process_group
            .lock()
            .as_ref()
            .map(|process_group| process_group.upgrade())
            .flatten()
        {
            process_group.pgid()
        } else {
            0
        }
    }

    pub fn process_group(&self) -> &Mutex<Option<Weak<ProcessGroup>>> {
        &self.process_group
    }

    /// add a child process
    pub fn add_child(&self, child: Arc<Process>) {
        debug!("process: {}, add child: {} ", self.pid(), child.pid());
        let child_pid = child.pid();
        self.children.lock().insert(child_pid, child);
    }

    fn set_parent(&self, parent: Weak<Process>) {
        let _ = self.parent.lock().insert(parent);
    }

    pub fn set_process_group(&self, process_group: Weak<ProcessGroup>) {
        if self.process_group.lock().is_none() {
            let _ = self.process_group.lock().insert(process_group);
        } else {
            todo!("We should do something with old group")
        }
    }

    /// create a new process group for the process and add it to globle table.
    /// Then set the process group for current process.
    fn create_and_set_process_group(self: &Arc<Self>) {
        let process_group = Arc::new(ProcessGroup::new(self.clone()));
        let pgid = process_group.pgid();
        self.set_process_group(Arc::downgrade(&process_group));
        table::add_process_group(pgid, process_group);
    }

    fn parent(&self) -> Option<Arc<Process>> {
        self.parent
            .lock()
            .as_ref()
            .map(|parent| parent.upgrade())
            .flatten()
    }

    /// Exit current process.
    /// Set the status of current process as Zombie and set exit code.
    /// Move all children to init process.
    /// Wake up the parent wait queue if parent is waiting for self.
    pub fn exit(&self, exit_code: i32) {
        self.status.lock().set_zombie();
        self.exit_code.store(exit_code, Ordering::Relaxed);
        // move children to the init process
        let current_process = Process::current();
        if !current_process.is_init_process() {
            let init_process = get_init_process();
            for (_, child_process) in self.children.lock().drain_filter(|_, _| true) {
                child_process.set_parent(Arc::downgrade(&init_process));
                init_process.add_child(child_process);
            }
        }

        if let Some(parent) = current_process.parent() {
            // set parent sig child
            let signal = Box::new(KernelSignal::new(SIGCHLD));
            parent.sig_queues().lock().enqueue(signal);
            // wake up parent waiting children, if any
            parent
                .waiting_children()
                .wake_all_on_condition(&current_process.pid(), |filter, pid| {
                    filter.contains_pid(*pid)
                });
        }
    }

    /// if the current process is init process
    pub fn is_init_process(&self) -> bool {
        self.pid == 0
    }

    /// start to run current process
    pub fn send_to_scheduler(self: &Arc<Self>) {
        self.task.send_to_scheduler();
    }

    /// yield the current process to allow other processes to run
    pub fn yield_now() {
        Task::yield_now();
    }

    /// returns the userspace
    pub fn user_space(&self) -> Option<&Arc<UserSpace>> {
        self.user_space.as_ref()
    }

    /// returns the vm space if the process does have, otherwise None
    pub fn vm_space(&self) -> Option<&VmSpace> {
        match self.user_space {
            None => None,
            Some(ref user_space) => Some(user_space.vm_space()),
        }
    }

    /// returns the user_vm
    pub fn user_vm(&self) -> Option<&UserVm> {
        self.user_vm.as_ref()
    }

    /// returns the user heap if the process does have, otherwise None
    pub fn user_heap(&self) -> Option<&UserHeap> {
        match self.user_vm {
            None => None,
            Some(ref user_vm) => Some(user_vm.user_heap()),
        }
    }

    /// returns the mmap area if the process does have, otherwise None
    pub fn mmap_area(&self) -> Option<&MmapArea> {
        match self.user_vm {
            None => None,
            Some(ref user_vm) => Some(user_vm.mmap_area()),
        }
    }

    /// Get child process with given pid
    pub fn get_child_by_pid(&self, pid: Pid) -> Option<Arc<Process>> {
        for (child_pid, child_process) in self.children.lock().iter() {
            if *child_pid == pid {
                return Some(child_process.clone());
            }
        }
        None
    }

    /// free zombie child with pid, returns the exit code of child process.
    /// remove process from process group.
    pub fn reap_zombie_child(&self, pid: Pid) -> i32 {
        let child_process = self.children.lock().remove(&pid).unwrap();
        assert!(child_process.status().lock().is_zombie());
        table::remove_process(child_process.pid());
        if let Some(process_group) = child_process.process_group().lock().as_ref() {
            if let Some(process_group) = process_group.upgrade() {
                process_group.remove_process(child_process.pid);
            }
        }
        child_process.exit_code()
    }

    /// Get any zombie child
    pub fn get_zombie_child(&self) -> Option<Arc<Process>> {
        for (_, child_process) in self.children.lock().iter() {
            if child_process.status().lock().is_zombie() {
                return Some(child_process.clone());
            }
        }
        None
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Relaxed)
    }

    /// whether the process has child process
    pub fn has_child(&self) -> bool {
        self.children.lock().len() != 0
    }

    pub fn filename(&self) -> Option<&CString> {
        self.filename.as_ref()
    }

    pub fn status(&self) -> &Mutex<ProcessStatus> {
        &self.status
    }

    pub fn sig_dispositions(&self) -> &Mutex<SigDispositions> {
        &self.sig_dispositions
    }

    pub fn sig_queues(&self) -> &Mutex<SigQueues> {
        &self.sig_queues
    }

    pub fn sig_mask(&self) -> &Mutex<SigMask> {
        &self.sig_mask
    }
}

/// Get the init process
pub fn get_init_process() -> Arc<Process> {
    let mut current_process = Process::current();
    while current_process.pid() != 0 {
        let process = current_process
            .parent
            .lock()
            .as_ref()
            .map(|current| current.upgrade())
            .flatten()
            .expect("[Internal Error] init process cannot be None");
        current_process = process;
    }
    current_process
}

/// allocate a new pid for new process
pub fn new_pid() -> Pid {
    PID_ALLOCATOR.fetch_add(1, Ordering::Release)
}
