// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::CpuSet,
    task::{Task, TaskOptions},
};

use super::{status::ThreadStatus, Thread};
use crate::{prelude::*, sched::priority::Priority};

/// The inner data of a kernel thread
pub struct KernelThread;

pub trait KernelThreadExt {
    /// Gets the kernel_thread structure
    fn as_kernel_thread(&self) -> Option<&KernelThread>;
    /// Creates a new kernel thread, and then run the thread.
    fn spawn_kernel_thread(thread_options: ThreadOptions) -> Arc<Thread> {
        let task = create_new_kernel_task(thread_options);
        let thread = Thread::borrow_from_task(&task).clone();
        thread.run();
        thread
    }
    /// join a kernel thread, returns if the kernel thread exit
    fn join(&self);
}

impl KernelThreadExt for Thread {
    fn as_kernel_thread(&self) -> Option<&KernelThread> {
        self.data().downcast_ref::<KernelThread>()
    }

    fn join(&self) {
        loop {
            if self.status().is_exited() {
                return;
            } else {
                Thread::yield_now();
            }
        }
    }
}

/// Creates a new task of kernel thread, **NOT** run the thread.
pub fn create_new_kernel_task(mut thread_options: ThreadOptions) -> Arc<Task> {
    let task_fn = thread_options.take_func();
    let thread_fn = move || {
        task_fn();
        // Ensures the thread is exit
        current_thread!().exit();
    };

    Arc::new_cyclic(|weak_task| {
        let thread = {
            let kernel_thread = KernelThread;
            let status = ThreadStatus::Init;
            let priority = thread_options.priority;
            let cpu_affinity = thread_options.cpu_affinity;
            Arc::new(Thread::new(
                weak_task.clone(),
                kernel_thread,
                status,
                priority,
                cpu_affinity,
            ))
        };

        TaskOptions::new(thread_fn).data(thread).build().unwrap()
    })
}

/// Options to create or spawn a new thread.
pub struct ThreadOptions {
    func: Option<Box<dyn Fn() + Send + Sync>>,
    priority: Priority,
    cpu_affinity: CpuSet,
}

impl ThreadOptions {
    pub fn new<F>(func: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        let cpu_affinity = CpuSet::new_full();
        Self {
            func: Some(Box::new(func)),
            priority: Priority::default(),
            cpu_affinity,
        }
    }

    pub fn func<F>(mut self, func: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.func = Some(Box::new(func));
        self
    }

    fn take_func(&mut self) -> Box<dyn Fn() + Send + Sync> {
        self.func.take().unwrap()
    }

    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    pub fn cpu_affinity(mut self, cpu_affinity: CpuSet) -> Self {
        self.cpu_affinity = cpu_affinity;
        self
    }
}
