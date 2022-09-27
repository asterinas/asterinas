use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

use alloc::ffi::CString;
use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use kxos_frame::{debug, task::Task, user::UserSpace, vm::VmSpace};
use spin::Mutex;

use crate::memory::mmap_area::MmapArea;
use crate::memory::user_heap::UserHeap;

use self::status::ProcessStatus;
use self::task::create_user_task_from_elf;
use self::user_vm_data::UserVm;

pub mod fifo_scheduler;
pub mod status;
pub mod task;
pub mod user_vm_data;

static PID_ALLOCATOR: AtomicUsize = AtomicUsize::new(0);

const CHILDREN_CAPACITY: usize = 16;

/// Process stands for a set of tasks that shares the same userspace.
/// Currently, we only support one task inside a process.
pub struct Process {
    // Immutable Part
    pid: usize,
    task: Arc<Task>,
    filename: Option<CString>,
    user_space: Option<Arc<UserSpace>>,
    user_vm: Option<UserVm>,

    // Mutable Part
    /// The exit code
    exit_code: AtomicI32,
    /// Process status
    status: Mutex<ProcessStatus>,
    /// Parent process
    parent: Mutex<Option<Weak<Process>>>,
    /// Children processes
    children: Mutex<Vec<Arc<Process>>>,
}

impl Process {
    /// returns the current process
    pub fn current() -> Arc<Process> {
        let task = Task::current();
        let process = task
            .data()
            .downcast_ref::<Weak<Process>>()
            .expect("[Internal Error] Task data should points to weak<process>");
        process
            .upgrade()
            .expect("[Internal Error] current process cannot be None")
    }

    /// create a new process(not schedule it)
    pub fn new(
        pid: usize,
        task: Arc<Task>,
        exec_filename: Option<CString>,
        user_vm: Option<UserVm>,
        user_space: Option<Arc<UserSpace>>,
    ) -> Self {
        let parent = if pid == 0 {
            debug!("Init process does not has parent");
            None
        } else {
            debug!("All process except init should have parent");
            let current_process = Process::current();
            Some(Arc::downgrade(&current_process))
        };
        let children = Vec::with_capacity(CHILDREN_CAPACITY);
        Self {
            pid,
            task,
            filename: exec_filename,
            user_space,
            user_vm,
            exit_code: AtomicI32::new(0),
            status: Mutex::new(ProcessStatus::Runnable),
            parent: Mutex::new(parent),
            children: Mutex::new(children),
        }
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

        Arc::new_cyclic(|weak_process_ref| {
            let weak_process = weak_process_ref.clone();
            let cloned_filename = Some(filename.clone());
            let task = create_user_task_from_elf(filename, elf_file_content, weak_process);
            let user_space = task.user_space().map(|user_space| user_space.clone());
            let user_vm = UserVm::new();
            Process::new(pid, task, cloned_filename, Some(user_vm), user_space)
        })
    }

    fn create_kernel_process<F>(task_fn: F) -> Arc<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let pid = new_pid();
        Arc::new_cyclic(|weak_process_ref| {
            let weak_process = weak_process_ref.clone();
            let task = Task::new(task_fn, weak_process, None).expect("spawn kernel task failed");
            Process::new(pid, task, None, None, None)
        })
    }

    /// returns the pid
    pub fn pid(&self) -> usize {
        self.pid
    }

    /// add a child process
    pub fn add_child(&self, child: Arc<Process>) {
        debug!("process: {}, add child: {} ", self.pid(), child.pid());
        self.children.lock().push(child);
    }

    fn set_parent(&self, parent: Weak<Process>) {
        let _ = self.parent.lock().insert(parent);
    }

    /// Set the exit code when calling exit or exit_group
    pub fn set_exit_code(&self, exit_code: i32) {
        self.exit_code.store(exit_code, Ordering::Relaxed);
    }

    /// Exit current process
    /// Set the status of current process as Zombie
    /// Move all children to init process
    pub fn exit(&self) {
        self.status.lock().set_zombie();
        // move children to the init process
        let current_process = Process::current();
        if !current_process.is_init_process() {
            let init_process = get_init_process();
            for child in self.children.lock().drain(..) {
                child.set_parent(Arc::downgrade(&init_process));
                init_process.add_child(child);
            }
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

    /// whether the process has child process
    pub fn has_child(&self) -> bool {
        self.children.lock().len() != 0
    }

    pub fn filename(&self) -> Option<&CString> {
        self.filename.as_ref()
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
pub fn new_pid() -> usize {
    PID_ALLOCATOR.fetch_add(1, Ordering::Release)
}
