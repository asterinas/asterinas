// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::CpuSet,
    task::{MutTaskInfo, Priority, SharedTaskInfo, Task, TaskContext, TaskOptions},
};

use super::{
    allocate_tid,
    status::{AtomicThreadStatus, ThreadStatus},
    thread_table, MutThreadInfo, MutThreadTaskData, ShareThreadTaskData, SharedThreadInfo,
    ThreadContext,
};
use crate::prelude::*;

/// The part of the kernel thread data that is shared between all threads.
pub struct SharedKernelThreadInfo;

/// The part of the  kernel thread data that is exclusive to the current thread.
pub struct MutKernelThreadInfo;

pub trait KernelThreadFn = Fn(
        &mut MutTaskInfo,
        &SharedTaskInfo,
        &mut MutThreadInfo,
        &SharedThreadInfo,
        &mut MutKernelThreadInfo,
        &SharedKernelThreadInfo,
    ) + 'static;

pub fn new_kernel(
    func: impl KernelThreadFn,
    priority: Priority,
    cpu_affinity: CpuSet,
) -> Arc<Task> {
    let kernel_task_entry = move |task_ctx_mut: &mut MutTaskInfo,
                                  task_ctx: &SharedTaskInfo,
                                  task_data_mut: &mut dyn Any,
                                  task_data: &(dyn Any + Send + Sync)| {
        let thread_data_mut = task_data_mut.downcast_mut::<MutThreadTaskData>().unwrap();
        let thread_data = task_data.downcast_ref::<ShareThreadTaskData>().unwrap();

        let thread_ctx_mut = &mut thread_data_mut.info;
        let thread_ctx = &thread_data.info;

        let kthread_ctx_mut = thread_data_mut
            .ext_data
            .downcast_mut::<MutKernelThreadInfo>()
            .unwrap();
        let kthread_ctx = thread_data
            .ext_data
            .downcast_ref::<SharedKernelThreadInfo>()
            .unwrap();

        func(
            task_ctx_mut,
            task_ctx,
            thread_ctx_mut,
            thread_ctx,
            kthread_ctx_mut,
            kthread_ctx,
        );

        // ensure the thread is exit
        thread_ctx_mut.exit(thread_ctx);
    };

    let mut_data = MutThreadTaskData {
        info: MutThreadInfo { _private: () },
        ext_data: Box::new(MutKernelThreadInfo),
    };

    let shared_data = ShareThreadTaskData {
        info: SharedThreadInfo {
            tid: allocate_tid(),
            status: AtomicThreadStatus::new(ThreadStatus::Init),
        },
        ext_data: Box::new(SharedKernelThreadInfo),
    };

    let thread = TaskOptions::new(kernel_task_entry)
        .shared_data(shared_data)
        .mut_data(mut_data)
        .priority(priority)
        .cpu_affinity(cpu_affinity)
        .build()
        .expect("Failed to build a user task");

    thread_table::add_thread(thread.clone());

    thread
}
