// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::CpuSet,
    task::{Task, TaskOptions},
};

use super::{oops, status::ThreadStatus, AsThread, Thread};
use crate::{prelude::*, sched::priority::Priority};

/// The inner data of a kernel thread.
struct KernelThread;

/// Options to create or spawn a new kernel thread.
pub struct ThreadOptions {
    func: Option<Box<dyn Fn() + Send + Sync>>,
    priority: Priority,
    cpu_affinity: CpuSet,
}

impl ThreadOptions {
    /// Creates the thread options with the thread function.
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

    /// Sets the priority of the new thread.
    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Sets the CPU affinity of the new thread.
    pub fn cpu_affinity(mut self, cpu_affinity: CpuSet) -> Self {
        self.cpu_affinity = cpu_affinity;
        self
    }
}

impl ThreadOptions {
    /// Builds a new kernel thread without running it immediately.
    pub fn build(mut self) -> Arc<Task> {
        let task_fn = self.func.take().unwrap();
        let thread_fn = move || {
            let _ = oops::catch_panics_as_oops(task_fn);
            // Ensure that the thread exits.
            current_thread!().exit();
        };

        Arc::new_cyclic(|weak_task| {
            let thread = {
                let kernel_thread = KernelThread;
                let status = ThreadStatus::Init;
                let priority = self.priority;
                let cpu_affinity = self.cpu_affinity;
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

    /// Builds a new kernel thread and runs it immediately.
    pub fn spawn(self) -> Arc<Thread> {
        let task = self.build();
        let thread = task.as_thread().unwrap().clone();
        thread.run();
        thread
    }
}
