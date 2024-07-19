// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::cell::RefCell;

use super::{
    scheduler::{fetch_task, GLOBAL_SCHEDULER},
    task::{context_switch, TaskContext},
    Task, TaskStatus,
};
use crate::{arch, cpu_local};

pub struct Processor {
    current: Option<Arc<Task>>,
    /// A temporary variable used in [`switch_to_task`] to avoid dropping `current` while running
    /// as `current`.
    prev_task: Option<Arc<Task>>,
    idle_task_ctx: TaskContext,
}

impl Processor {
    pub const fn new() -> Self {
        Self {
            current: None,
            prev_task: None,
            idle_task_ctx: TaskContext::new(),
        }
    }
    fn get_idle_task_ctx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_ctx as *mut _
    }
    pub fn take_current(&mut self) -> Option<Arc<Task>> {
        self.current.take()
    }
    pub fn current(&self) -> Option<Arc<Task>> {
        self.current.as_ref().map(Arc::clone)
    }
    pub fn set_current_task(&mut self, task: Arc<Task>) {
        self.current = Some(task.clone());
    }
}

cpu_local! {
    static PROCESSOR: RefCell<Processor> = RefCell::new(Processor::new());
}

/// Retrieves the current task running on the processor.
pub fn current_task() -> Option<Arc<Task>> {
    PROCESSOR.borrow_irq_disabled().borrow().current()
}

pub(crate) fn get_idle_task_ctx_ptr() -> *mut TaskContext {
    PROCESSOR
        .borrow_irq_disabled()
        .borrow_mut()
        .get_idle_task_ctx_ptr()
}

/// Calls this function to switch to other task by using GLOBAL_SCHEDULER
pub fn schedule() {
    if let Some(task) = fetch_task() {
        switch_to_task(task);
    }
}

/// Preempts the `task`.
///
/// TODO: This interface of this method is error prone.
/// The method takes an argument for the current task to optimize its efficiency,
/// but the argument provided by the caller may not be the current task, really.
/// Thus, this method should be removed or reworked in the future.
pub fn preempt(task: &Arc<Task>) {
    // TODO: Refactor `preempt` and `schedule`
    // after the Atomic mode and `might_break` is enabled.
    let mut scheduler = GLOBAL_SCHEDULER.lock_irq_disabled();
    if !scheduler.should_preempt(task) {
        return;
    }
    let Some(next_task) = scheduler.dequeue() else {
        return;
    };
    drop(scheduler);
    switch_to_task(next_task);
}

/// Calls this function to switch to other task
///
/// if current task is none, then it will use the default task context and it will not return to this function again
///
/// if current task status is exit, then it will not add to the scheduler
///
/// before context switch, current task will switch to the next task
fn switch_to_task(next_task: Arc<Task>) {
    if !PREEMPT_COUNT.is_preemptive() {
        panic!(
            "Calling schedule() while holding {} locks",
            PREEMPT_COUNT.num_locks()
        );
    }

    let current_task_ctx_ptr = match current_task() {
        None => get_idle_task_ctx_ptr(),
        Some(current_task) => {
            let ctx_ptr = current_task.ctx().get();

            let mut task_inner = current_task.inner_exclusive_access();

            debug_assert_ne!(task_inner.task_status, TaskStatus::Sleeping);
            if task_inner.task_status == TaskStatus::Runnable {
                drop(task_inner);
                GLOBAL_SCHEDULER.lock_irq_disabled().enqueue(current_task);
            } else if task_inner.task_status == TaskStatus::Sleepy {
                task_inner.task_status = TaskStatus::Sleeping;
            }

            ctx_ptr
        }
    };

    let next_task_ctx_ptr = next_task.ctx().get().cast_const();

    if let Some(next_user_space) = next_task.user_space() {
        next_user_space.vm_space().activate();
    }

    // Change the current task to the next task.
    {
        let processor_guard = PROCESSOR.borrow_irq_disabled();
        let mut processor = processor_guard.borrow_mut();

        // We cannot directly overwrite `current` at this point. Since we are running as `current`,
        // we must avoid dropping `current`. Otherwise, the kernel stack may be unmapped, leading
        // to soundness problems.
        let old_current = processor.current.replace(next_task);
        processor.prev_task = old_current;
    }

    // SAFETY:
    // 1. `ctx` is only used in `schedule()`. We have exclusive access to both the current task
    //    context and the next task context.
    // 2. The next task context is a valid task context.
    unsafe {
        // This function may not return, for example, when the current task exits. So make sure
        // that all variables on the stack can be forgotten without causing resource leakage.
        context_switch(current_task_ctx_ptr, next_task_ctx_ptr);
    }

    // Now it's fine to drop `prev_task`. However, we choose not to do this because it is not
    // always possible. For example, `context_switch` can switch directly to the entry point of the
    // next task. Not dropping is just fine because the only consequence is that we delay the drop
    // to the next task switching.
}

static PREEMPT_COUNT: PreemptInfo = PreemptInfo::new();

/// Currently, it only holds the number of preemption locks held by the
/// current CPU. When it has a non-zero value, the CPU cannot call
/// [`schedule()`].
///
/// For per-CPU preemption lock count, we cannot afford two non-atomic
/// operations to increment and decrement the count. The [`crate::cpu_local`]
/// implementation is free to read the base register and then calculate the
/// address of the per-CPU variable using an additional instruction. Interrupts
/// can happen between the address calculation and modification to that
/// address. If the task is preempted to another CPU by this interrupt, the
/// count of the original CPU will be mistakenly modified. To avoid this, we
/// introduce [`crate::arch::cpu::local::preempt_lock_count`]. For x86_64 we
/// can implement this using one instruction. In other less expressive
/// architectures, we may need to disable interrupts.
///
/// Also, the preemption count is reserved in the `.cpu_local` section
/// specified in the linker script. The reason is that we need to access the
/// preemption count before we can copy the section for application processors.
/// So, the preemption count is not copied from bootstrap processor's section
/// as the initialization. Instead it is initialized to zero for application
/// processors.
struct PreemptInfo {}

impl PreemptInfo {
    const fn new() -> Self {
        Self {}
    }

    fn increase_num_locks(&self) {
        arch::cpu::local::preempt_lock_count::inc();
    }

    fn decrease_num_locks(&self) {
        arch::cpu::local::preempt_lock_count::dec();
    }

    fn is_preemptive(&self) -> bool {
        arch::cpu::local::preempt_lock_count::get() == 0
    }

    fn num_locks(&self) -> usize {
        arch::cpu::local::preempt_lock_count::get() as usize
    }
}

/// A guard for disable preempt.
#[clippy::has_significant_drop]
#[must_use]
pub struct DisablePreemptGuard {
    // This private field prevents user from constructing values of this type directly.
    _private: (),
}

impl !Send for DisablePreemptGuard {}

impl DisablePreemptGuard {
    fn new() -> Self {
        PREEMPT_COUNT.increase_num_locks();
        Self { _private: () }
    }

    /// Transfer this guard to a new guard.
    /// This guard must be dropped after this function.
    pub fn transfer_to(&self) -> Self {
        disable_preempt()
    }
}

impl Drop for DisablePreemptGuard {
    fn drop(&mut self) {
        PREEMPT_COUNT.decrease_num_locks();
    }
}

/// Disables preemption.
pub fn disable_preempt() -> DisablePreemptGuard {
    DisablePreemptGuard::new()
}
