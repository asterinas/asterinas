// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::{
    cell::RefCell,
    ops::DerefMut,
    sync::atomic::{AtomicBool, Ordering},
};

use intrusive_collections::{intrusive_adapter, LinkedList, LinkedListAtomicLink};
use ostd::{cpu::local::CpuLocal, cpu_local, trap};

use super::{
    softirq_id::{TASKLESS_SOFTIRQ_ID, TASKLESS_URGENT_SOFTIRQ_ID},
    SoftIrqLine,
};

/// `Taskless` represents a _taskless_ job whose execution is deferred to a later time.
///
/// # Overview
///
/// `Taskless` provides one "bottom half" mechanism for interrupt handling.
/// With `Taskless`, one can defer the execution of certain logic
/// that would have been otherwise executed in interrupt handlers.
/// `Taskless` makes interrupt handlers finish more quickly,
/// thereby minimizing the periods of time when the interrupts are disabled.
///
/// `Taskless` executes the deferred jobs via the softirq mechanism,
/// rather than doing them with `Task`s.
/// As such, these deferred, taskless jobs can be executed within only a small delay,
/// after the execution of an interrupt handler that schedules the taskless jobs.
/// As the taskless jobs are not executed in the task context,
/// they are not allowed to sleep.
///
/// An `Taskless` instance may be scheduled to run multiple times,
/// but it is guaranteed that a single taskless job will not be run concurrently.
/// Also, a taskless job will not be preempted by another.
/// This makes the programming of a taskless job simpler.
/// Different taskless jobs are allowed to run concurrently.
/// Once a taskless has entered the execution state, it can be scheduled again.
///
/// # Example
///
/// Users can create a `Taskless` and schedule it at any place.
/// ```rust
/// #use ostd::softirq::Taskless;
///
/// #fn my_func() {}
///
/// let taskless = Taskless::new(my_func);
/// // This taskless job will be executed in softirq context soon.
/// taskless.schedule();
///
/// ```
pub struct Taskless {
    /// Whether the taskless job has been scheduled.
    is_scheduled: AtomicBool,
    /// Whether the taskless job is running.
    is_running: AtomicBool,
    /// The function that will be called when executing this taskless job.
    callback: Box<RefCell<dyn FnMut() + Send + Sync + 'static>>,
    /// Whether this `Taskless` is disabled.
    #[allow(unused)]
    is_disabled: AtomicBool,
    link: LinkedListAtomicLink,
}

intrusive_adapter!(TasklessAdapter = Arc<Taskless>: Taskless { link: LinkedListAtomicLink });

cpu_local! {
    static TASKLESS_LIST: RefCell<LinkedList<TasklessAdapter>> = RefCell::new(LinkedList::new(TasklessAdapter::NEW));
    static TASKLESS_URGENT_LIST: RefCell<LinkedList<TasklessAdapter>> = RefCell::new(LinkedList::new(TasklessAdapter::NEW));
}

impl Taskless {
    /// Creates a new `Taskless` instance with its callback function.
    #[allow(unused)]
    pub fn new<F>(callback: F) -> Arc<Self>
    where
        F: FnMut() + Send + Sync + 'static,
    {
        // Since the same taskless will not be executed concurrently,
        // it is safe to use a `RefCell` here though the `Taskless` will
        // be put into an `Arc`.
        #[allow(clippy::arc_with_non_send_sync)]
        Arc::new(Self {
            is_scheduled: AtomicBool::new(false),
            is_running: AtomicBool::new(false),
            callback: Box::new(RefCell::new(callback)),
            is_disabled: AtomicBool::new(false),
            link: LinkedListAtomicLink::new(),
        })
    }

    /// Schedules this taskless job and it will be executed in later time.
    ///
    /// If the taskless job has been scheduled, this function will do nothing.
    #[allow(unused)]
    pub fn schedule(self: &Arc<Self>) {
        do_schedule(self, &TASKLESS_LIST);
        SoftIrqLine::get(TASKLESS_SOFTIRQ_ID).raise();
    }

    /// Schedules this taskless job and it will be executed urgently
    /// in softirq context.
    ///
    /// If the taskless job has been scheduled, this function will do nothing.
    #[allow(unused)]
    pub fn schedule_urgent(self: &Arc<Self>) {
        do_schedule(self, &TASKLESS_URGENT_LIST);
        SoftIrqLine::get(TASKLESS_URGENT_SOFTIRQ_ID).raise();
    }

    /// Enables this `Taskless` so that it can be executed once it has been scheduled.
    ///
    /// A new `Taskless` is enabled by default.
    #[allow(unused)]
    pub fn enable(&self) {
        self.is_disabled.store(false, Ordering::Release);
    }

    /// Disables this `Taskless` so that it can not be scheduled. Note that if the `Taskless`
    /// has been scheduled, it can still continue to complete this job.
    #[allow(unused)]
    pub fn disable(&self) {
        self.is_disabled.store(true, Ordering::Release);
    }
}

#[allow(unused)]
fn do_schedule(
    taskless: &Arc<Taskless>,
    taskless_list: &'static CpuLocal<RefCell<LinkedList<TasklessAdapter>>>,
) {
    if taskless.is_disabled.load(Ordering::Acquire) {
        return;
    }
    if taskless
        .is_scheduled
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let irq_guard = trap::disable_local();
    taskless_list
        .get_with(&irq_guard)
        .borrow_mut()
        .push_front(taskless.clone());
}

pub(super) fn init() {
    SoftIrqLine::get(TASKLESS_URGENT_SOFTIRQ_ID)
        .enable(|| taskless_softirq_handler(&TASKLESS_URGENT_LIST, TASKLESS_URGENT_SOFTIRQ_ID));
    SoftIrqLine::get(TASKLESS_SOFTIRQ_ID)
        .enable(|| taskless_softirq_handler(&TASKLESS_LIST, TASKLESS_SOFTIRQ_ID));
}

/// Executes the pending taskless jobs in the input `taskless_list`.
///
/// This function will retrieve each `Taskless` in the input `taskless_list`
/// and leave it empty. If a `Taskless` is running then this function will
/// ignore it and jump to the next `Taskless`, then put it to the input `taskless_list`.
///
/// If the `Taskless` is ready to be executed, it will be set to not scheduled
/// and can be scheduled again.
fn taskless_softirq_handler(
    taskless_list: &'static CpuLocal<RefCell<LinkedList<TasklessAdapter>>>,
    softirq_id: u8,
) {
    let mut processing_list = {
        let irq_guard = trap::disable_local();
        let guard = taskless_list.get_with(&irq_guard);
        let mut list_mut = guard.borrow_mut();
        LinkedList::take(list_mut.deref_mut())
    };

    while let Some(taskless) = processing_list.pop_back() {
        if taskless
            .is_running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            let irq_guard = trap::disable_local();
            taskless_list
                .get_with(&irq_guard)
                .borrow_mut()
                .push_front(taskless);
            SoftIrqLine::get(softirq_id).raise();
            continue;
        }

        taskless.is_scheduled.store(false, Ordering::Release);

        // The same taskless will not be executing concurrently, so it is safe to
        // do `borrow_mut` here.
        (taskless.callback.borrow_mut())();
        taskless.is_running.store(false, Ordering::Release);
    }
}

#[cfg(ktest)]
mod test {
    use core::sync::atomic::AtomicUsize;

    use ostd::prelude::*;

    use super::*;

    fn init() {
        static DONE: AtomicBool = AtomicBool::new(false);
        if !DONE.load(Ordering::SeqCst) {
            let _ = super::super::init();
            DONE.store(true, Ordering::SeqCst);
        }
    }

    #[ktest]
    fn schedule_taskless() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        const SCHEDULE_TIMES: usize = 10;

        fn add_counter() {
            COUNTER.fetch_add(1, Ordering::Relaxed);
        }

        init();
        let taskless = Taskless::new(add_counter);
        let mut counter = 0;

        // Schedule this taskless for `SCHEDULE_TIMES`.
        while !taskless.is_scheduled.load(Ordering::Acquire) {
            taskless.schedule();
            counter += 1;
            if counter == SCHEDULE_TIMES {
                break;
            }
        }

        // Wait for all taskless having finished.
        while taskless.is_running.load(Ordering::Acquire)
            || taskless.is_scheduled.load(Ordering::Acquire)
        {
            core::hint::spin_loop()
        }

        assert_eq!(counter, COUNTER.load(Ordering::Relaxed));
    }
}
