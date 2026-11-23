// SPDX-License-Identifier: MPL-2.0

//! Thread implementation.

use core::sync::atomic::{AtomicBool, Ordering};

use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{
    cpu::{AtomicCpuSet, CpuId, CpuSet},
    irq::DisabledLocalIrqGuard,
    pmu,
    task::Task,
};

use self::{kernel_thread::AsKernelThread, stats::CONTEXT_SWITCH_COUNTER};
use crate::{
    events::perf::PerfContext,
    prelude::*,
    sched::{SchedAttr, SchedPolicy},
};

pub(crate) mod exception;
pub(crate) mod kernel_thread;
pub(crate) mod oops;
mod softirqd;
mod stats;
pub(crate) mod task;
pub(crate) mod work_queue;

pub(crate) use self::stats::collect_context_switch_count;

pub(crate) type Tid = u32;

fn pre_schedule_handler(irq_guard: &DisabledLocalIrqGuard) {
    let Some(task) = Task::current() else {
        return;
    };
    if pmu::HAS_BACKEND
        && let Some(perf) = task.as_thread().and_then(|thread| thread.perf())
    {
        perf.on_schedule_out(irq_guard);
    }
    let Some(thread_local) = task.as_thread_local() else {
        return;
    };

    thread_local.supp_user_context().before_schedule(irq_guard);
}

fn post_schedule_handler() {
    // No races because preemption shouldn't happen in pre-/post-schedule handlers.
    CONTEXT_SWITCH_COUNTER
        .get()
        .unwrap()
        .add_on_cpu(CpuId::current_racy(), 1);

    let task = Task::current().unwrap();
    if let Some(thread_local) = task.as_thread_local() {
        let vmar = thread_local.vmar().borrow();
        if let Some(vmar) = vmar.as_ref() {
            vmar.vm_space().activate()
        }
    } else if let Some(vmar) = task
        .as_kernel_thread()
        .and_then(|kernel_thread| kernel_thread.vmar())
    {
        vmar.vm_space().activate();
    }
    if pmu::HAS_BACKEND
        && let Some(perf) = task.as_thread().and_then(|thread| thread.perf())
    {
        perf.on_schedule_in();
    }
}

pub(super) fn init() {
    CONTEXT_SWITCH_COUNTER.call_once(PerCpuCounter::new);
    ostd::task::inject_pre_schedule_handler(pre_schedule_handler);
    ostd::task::inject_post_schedule_handler(post_schedule_handler);
    ostd::mm::fault::inject_user_page_fault_handler(exception::page_fault_handler);
}

pub(super) fn init_in_first_kthread() {
    work_queue::init_in_first_kthread();
    softirqd::init_in_first_kthread();
}

/// A thread is a wrapper on top of a task.
#[derive(Debug)]
pub(crate) struct Thread {
    // Immutable part:
    //
    /// Low-level task.
    task: Weak<Task>,
    /// POSIX thread information or kernel thread information.
    data: Box<dyn Send + Sync + Any>,

    // Mutable part:
    //
    /// Thread status.
    is_exited: AtomicBool,
    /// Thread CPU affinity.
    cpu_affinity: AtomicCpuSet,
    /// Thread scheduling attribute.
    sched_attr: SchedAttr,
    perf: Option<Arc<PerfContext>>,
}

impl Thread {
    /// Never call these function directly
    pub(crate) fn new(
        task: Weak<Task>,
        data: impl Send + Sync + Any,
        cpu_affinity: CpuSet,
        sched_policy: SchedPolicy,
    ) -> Self {
        Thread {
            task,
            data: Box::new(data),
            is_exited: AtomicBool::new(false),
            cpu_affinity: AtomicCpuSet::new(cpu_affinity),
            sched_attr: SchedAttr::new(sched_policy),
            perf: pmu::HAS_BACKEND.then(|| Arc::new(PerfContext::new())),
        }
    }

    /// Returns the current thread.
    ///
    /// This function returns `None` if the current task is not associated with
    /// a thread, or if called within the bootstrap context.
    pub(crate) fn current() -> Option<Arc<Self>> {
        Task::current()?.as_thread().cloned()
    }

    /// Returns the task associated with this thread.
    #[expect(dead_code)]
    pub(crate) fn task(&self) -> Arc<Task> {
        self.task.upgrade().unwrap()
    }

    /// Runs this thread at once.
    #[track_caller]
    pub(crate) fn run(&self) {
        self.task.upgrade().unwrap().run();
    }

    /// Returns whether the thread is exited.
    pub(crate) fn is_exited(&self) -> bool {
        self.is_exited.load(Ordering::Acquire)
    }

    /// Returns the context that owns this thread's performance events.
    ///
    /// Event files can retain the context beyond thread exit to read final counts.
    /// Builds without a hardware PMU backend return `None`.
    pub(crate) fn perf(&self) -> Option<&Arc<PerfContext>> {
        self.perf.as_ref()
    }

    pub(super) fn exit(&self) {
        if let Some(perf) = self.perf() {
            perf.on_exit();
        }
        self.is_exited.store(true, Ordering::Release);
    }

    /// Returns the reference to the atomic CPU affinity.
    pub(crate) fn atomic_cpu_affinity(&self) -> &AtomicCpuSet {
        &self.cpu_affinity
    }

    pub(crate) fn sched_attr(&self) -> &SchedAttr {
        &self.sched_attr
    }

    /// Yields the execution to another thread.
    ///
    /// This method will return once the current thread is scheduled again.
    #[track_caller]
    pub(crate) fn yield_now() {
        Task::yield_now()
    }

    /// Joins the execution of the thread.
    ///
    /// This method will return after the thread exits.
    #[cfg_attr(not(ktest), expect(dead_code))]
    #[track_caller]
    pub(crate) fn join(&self) {
        while !self.is_exited() {
            Self::yield_now();
        }
    }

    /// Returns the associated data.
    pub(crate) fn data(&self) -> &(dyn Send + Sync + Any) {
        &*self.data
    }
}

/// A trait to provide the `as_thread` method for tasks.
pub(crate) trait AsThread {
    /// Returns the associated [`Thread`].
    fn as_thread(&self) -> Option<&Arc<Thread>>;
}

impl AsThread for Task {
    fn as_thread(&self) -> Option<&Arc<Thread>> {
        self.data().downcast_ref::<Arc<Thread>>()
    }
}
