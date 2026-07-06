// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::CpuSet,
    task::{Task, TaskOptions},
};

use crate::{
    prelude::*,
    process::posix_thread::{SuppUserContext, ThreadLocal},
    sched::{Nice, SchedPolicy},
    thread::{AsThread, Thread, oops},
};

struct IoUringThread;

pub(super) struct IoUringThreadOptions {
    func: Option<Box<dyn FnOnce() + Send>>,
    thread_local: ThreadLocal,
    cpu_affinity: CpuSet,
    sched_policy: SchedPolicy,
}

impl IoUringThreadOptions {
    pub(super) fn clone_thread_local(thread_local: &ThreadLocal) -> Result<ThreadLocal> {
        let vmar = {
            let vmar_ref = thread_local.vmar().borrow();
            let Some(vmar) = vmar_ref.as_ref() else {
                return_errno_with_message!(Errno::EFAULT, "the current thread has no user VMAR");
            };
            vmar.clone_handle()
        };
        let file_table = thread_local.borrow_file_table().unwrap().clone();
        let fs = thread_local.borrow_fs().clone();
        let user_ns = thread_local.borrow_user_ns().clone();
        let ns_proxy = thread_local.borrow_ns_proxy().unwrap().clone();

        Ok(ThreadLocal::new(
            0,
            0,
            vmar,
            file_table,
            fs,
            SuppUserContext::new(),
            user_ns,
            ns_proxy,
        ))
    }

    pub(super) fn new<F>(func: F, thread_local: ThreadLocal) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self {
            func: Some(Box::new(func)),
            thread_local,
            cpu_affinity: CpuSet::new_full(),
            sched_policy: SchedPolicy::Fair(Nice::default()),
        }
    }

    pub(super) fn cpu_affinity(mut self, cpu_affinity: CpuSet) -> Self {
        self.cpu_affinity = cpu_affinity;
        self
    }

    #[track_caller]
    pub(super) fn spawn(self) -> Arc<Thread> {
        let task = self.build();
        let thread = task.as_thread().unwrap().clone();
        thread.run();
        thread
    }

    fn build(mut self) -> Arc<Task> {
        let task_fn = self.func.take().unwrap();
        let thread_fn = move || {
            let _ = oops::catch_panics_as_oops(task_fn);
            current_thread!().exit();
        };

        Arc::new_cyclic(|weak_task| {
            let thread = {
                let cpu_affinity = self.cpu_affinity;
                let sched_policy = self.sched_policy;
                Arc::new(Thread::new(
                    weak_task.clone(),
                    IoUringThread,
                    cpu_affinity,
                    sched_policy,
                ))
            };

            TaskOptions::new(thread_fn)
                .data(thread)
                .local_data(self.thread_local)
                .build()
                .unwrap()
        })
    }
}
