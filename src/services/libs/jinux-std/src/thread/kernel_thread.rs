use jinux_frame::task::Task;

use crate::{prelude::*, process::Process};

use super::{allocate_tid, status::ThreadStatus, thread_table, Thread};
pub struct KernelThread {
    process: Weak<Process>,
}

impl KernelThread {
    pub fn new(process: Weak<Process>) -> Self {
        Self { process }
    }

    pub fn process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }
}

pub trait KernelThreadExt {
    fn is_kernel_thread(&self) -> bool;
    fn kernel_thread(&self) -> &KernelThread;
    fn new_kernel_thread<F>(task_fn: F, process: Weak<Process>) -> Arc<Self>
    where
        F: Fn() + Send + Sync + 'static;
}

impl KernelThreadExt for Thread {
    fn is_kernel_thread(&self) -> bool {
        self.data().downcast_ref::<KernelThread>().is_some()
    }

    fn kernel_thread(&self) -> &KernelThread {
        self.data().downcast_ref::<KernelThread>().unwrap()
    }

    fn new_kernel_thread<F>(task_fn: F, process: Weak<Process>) -> Arc<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let tid = allocate_tid();
        let thread = Arc::new_cyclic(|thread_ref| {
            let weal_thread = thread_ref.clone();
            let task = Task::new(task_fn, weal_thread, None).unwrap();
            let status = ThreadStatus::Init;
            let kernel_thread = KernelThread::new(process);
            Thread::new(tid, task, kernel_thread, status)
        });
        thread_table::add_thread(thread.clone());
        thread
    }
}
