// SPDX-License-Identifier: MPL-2.0

use aster_frame::cpu::CpuSet;
use aster_frame::task::{Priority, TaskOptions};

use crate::prelude::*;

use super::{allocate_tid, status::ThreadStatus, thread_table, Thread};

/// The inner data of a kernel thread
pub struct KernelThread;

pub trait KernelThreadExt {
    /// get the kernel_thread structure
    fn as_kernel_thread(&self) -> Option<&KernelThread>;
    /// create a new kernel thread structure, **NOT** run the thread.
    fn new_kernel_thread(thread_options: ThreadOptions) -> Arc<Thread>;
    /// create a new kernel thread structure, and then run the thread.
    fn spawn_kernel_thread(thread_options: ThreadOptions) -> Arc<Thread> {
        let thread = Self::new_kernel_thread(thread_options);
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

    fn new_kernel_thread(mut thread_options: ThreadOptions) -> Arc<Self> {
        let task_fn = thread_options.take_func();
        let thread_fn = move || {
            task_fn();
            let current_thread = current_thread!();
            // ensure the thread is exit
            current_thread.exit();
        };
        let tid = allocate_tid();
        let thread = Arc::new_cyclic(|thread_ref| {
            let weal_thread = thread_ref.clone();
            let task = TaskOptions::new(thread_fn)
                .data(weal_thread)
                .build()
                .unwrap();
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
            priority: Priority::normal(),
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
