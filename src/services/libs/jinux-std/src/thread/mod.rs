//! Posix thread implementation

use crate::{
    prelude::*,
    process::{elf::load_elf_to_root_vmar, Process},
    rights::Full,
    vm::vmar::Vmar,
};
use jinux_frame::{cpu::CpuContext, task::Task, user::UserSpace};

use self::task::create_new_user_task;

pub mod task;

pub type Tid = i32;

/// A thread is a wrapper on top of task.
pub struct Thread {
    /// Thread id
    tid: Tid,
    /// Low-level info
    task: Arc<Task>,
    /// The process. FIXME: should we store the process info here?
    process: Weak<Process>,
}

impl Thread {
    pub fn new_user_thread_from_elf(
        root_vmar: &Vmar<Full>,
        filename: CString,
        elf_file_content: &'static [u8],
        process: Weak<Process>,
        tid: Tid,
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Arc<Self> {
        let elf_load_info =
            load_elf_to_root_vmar(filename, elf_file_content, &root_vmar, argv, envp)
                .expect("Load Elf failed");
        let vm_space = root_vmar.vm_space().clone();
        let mut cpu_ctx = CpuContext::default();
        cpu_ctx.set_rip(elf_load_info.entry_point());
        cpu_ctx.set_rsp(elf_load_info.user_stack_top());
        let user_space = Arc::new(UserSpace::new(vm_space, cpu_ctx));
        Thread::new_user_thread(tid, user_space, process)
    }

    pub fn new_user_thread(
        tid: Tid,
        user_space: Arc<UserSpace>,
        process: Weak<Process>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|thread_ref| {
            let task = create_new_user_task(user_space, thread_ref.clone());
            Thread { tid, task, process }
        })
    }

    pub fn new_kernel_thread<F>(tid: Tid, task_fn: F, process: Weak<Process>) -> Arc<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        Arc::new_cyclic(|thread_ref| {
            let weal_thread = thread_ref.clone();
            let task = Task::new(task_fn, weal_thread, None).unwrap();
            Thread { tid, task, process }
        })
    }

    pub fn current() -> Arc<Self> {
        let task = Task::current();
        let thread = task
            .data()
            .downcast_ref::<Weak<Thread>>()
            .expect("[Internal Error] task data should points to weak<process>");
        thread
            .upgrade()
            .expect("[Internal Error] current process cannot be None")
    }

    pub fn process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }

    /// Add inner task to the run queue of scheduler. Note this does not means the thread will run at once.
    pub fn run(&self) {
        self.task.run();
    }

    pub fn yield_now() {
        Task::yield_now()
    }
}
