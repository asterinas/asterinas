use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Arc;
// use kxos_frame::{sync::SpinLock, task::Task, user::UserSpace};
use kxos_frame::task::Task;

use self::task::spawn_user_task_from_elf;

pub mod fifo_scheduler;
pub mod task;

// static PROCESSES: SpinLock<BTreeMap<usize, Arc<Process>>> = SpinLock::new(BTreeMap::new());
static PID_ALLOCATOR: AtomicUsize = AtomicUsize::new(0);

/// Process stands for a set of tasks that shares the same userspace.
/// Currently, we only support one task inside a process.
pub struct Process {
    pid: usize,
    task: Arc<Task>,
    exit_code: i32,
    // user_space: Option<Arc<UserSpace>>,
    // TODO: childs, parent, files,
}

impl Process {
    pub fn spawn_from_elf(elf_file_content: &[u8]) -> Self {
        let pid = new_pid();
        let task = spawn_user_task_from_elf(elf_file_content);
        let exit_code = 0;
        Self {
            pid,
            task,
            exit_code,
        }
    }

    pub fn spawn_kernel_task<F>(task_fn: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        let pid = new_pid();
        let task = Task::spawn(task_fn, pid, None).expect("spawn kernel task failed");
        let exit_code = 0;
        Self {
            pid,
            task,
            exit_code,
        }
    }

    pub fn pid(&self) -> usize {
        self.pid
    }
}

/// create a new pid for new process
fn new_pid() -> usize {
    PID_ALLOCATOR.fetch_add(1, Ordering::Release)
}
