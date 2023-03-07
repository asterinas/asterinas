use jinux_frame::task::Task;

use crate::prelude::*;

use super::{allocate_tid, status::ThreadStatus, thread_table, Thread};

/// The inner data of a kernel thread
pub struct KernelThread;

pub trait KernelThreadExt {
    /// get the kernel_thread structure
    fn as_kernel_thread(&self) -> Option<&KernelThread>;
    /// create a new kernel thread structure, **NOT** run the thread.
    fn new_kernel_thread<F>(task_fn: F) -> Arc<Thread>
    where
        F: Fn() + Send + Sync + 'static;
    /// create a new kernel thread structure, and then run the thread.
    fn spawn_kernel_thread<F>(task_fn: F) -> Arc<Thread>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let thread = Self::new_kernel_thread(task_fn);
        thread.run();
        thread
    }
    /// join a kernel thread
    fn join(&self);
}

impl KernelThreadExt for Thread {
    fn as_kernel_thread(&self) -> Option<&KernelThread> {
        self.data().downcast_ref::<KernelThread>()
    }

    fn new_kernel_thread<F>(task_fn: F) -> Arc<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {

        let thread_fn = move || {
            task_fn();
            let current_thread = current_thread!();
            // ensure the thread is exit
            current_thread.exit();
        };
        let tid = allocate_tid();
        let thread = Arc::new_cyclic(|thread_ref| {
            let weal_thread = thread_ref.clone();
            let task = Task::new(thread_fn, weal_thread, None).unwrap();
            let status = ThreadStatus::Init;
            let kernel_thread = KernelThread;
            Thread::new(tid, task, kernel_thread, status)
        });
        thread_table::add_thread(thread.clone());
        thread
    }

    fn join(&self) {
        loop {
            let status = self.status.lock();
            if status.is_exited() {
                return;
            } else {
                drop(status);
                Thread::yield_now();
            }
        }
    }
}
