// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::task::{context_switch, Task, TaskContext};
use crate::{cpu::local::PREEMPT_INFO, cpu_local_cell};

cpu_local_cell! {
    /// The `Arc<Task>` (casted by [`Arc::into_raw`]) that is the current task.
    static CURRENT_TASK_PTR: *const Task = core::ptr::null();
    /// The previous task on the processor before switching to the current task.
    /// It is used for delayed resource release since it would be the current
    /// task's job to recycle the previous resources.
    static PREVIOUS_TASK_PTR: *const Task = core::ptr::null();
    /// An unsafe cell to store the context of the bootstrap code.
    static BOOTSTRAP_CONTEXT: TaskContext = TaskContext::new();
}

/// Retrieves a reference to the current task running on the processor.
///
/// It returns `None` if the function is called in the bootstrap context.
pub(super) fn current_task() -> Option<Arc<Task>> {
    let ptr = CURRENT_TASK_PTR.load();
    if ptr.is_null() {
        return None;
    }
    // SAFETY: The pointer is set by `switch_to_task` and is guaranteed to be
    // built with `Arc::into_raw`.
    let restored = unsafe { Arc::from_raw(ptr) };
    // To let the `CURRENT_TASK_PTR` still own the task, we clone and forget it
    // to increment the reference count.
    let _ = core::mem::ManuallyDrop::new(restored.clone());
    Some(restored)
}

/// Calls this function to switch to other task
///
/// If current task is none, then it will use the default task context and it
/// will not return to this function again.
///
/// # Panics
///
/// This function will panic if called while holding preemption locks or with
/// local IRQ disabled.
pub(super) fn switch_to_task(next_task: Arc<Task>) {
    let preemt_count = PREEMPT_COUNT.get();
    if preemt_count != 0 {
        panic!("Switching task while holding {} locks", preemt_count);
    }

    assert!(
        crate::arch::irq::is_local_enabled(),
        "Switching task with local IRQ disabled"
    );

    let irq_guard = crate::trap::disable_local();

    let current_task_ptr = CURRENT_TASK_PTR.load();
    let current_task_ctx_ptr = if current_task_ptr.is_null() {
        // SAFETY: Interrupts are disabled, so the pointer is safe to be fetched.
        unsafe { BOOTSTRAP_CONTEXT.as_ptr_mut() }
    } else {
        // SAFETY: The pointer is not NULL and set as the current task.
        let cur_task_arc = unsafe {
            let restored = Arc::from_raw(current_task_ptr);
            let _ = core::mem::ManuallyDrop::new(restored.clone());
            restored
        };
        let ctx_ptr = cur_task_arc.ctx().get();

        ctx_ptr
    };

    let next_task_ctx_ptr = next_task.ctx().get().cast_const();
    if let Some(next_user_space) = next_task.user_space() {
        next_user_space.vm_space().activate();
    }

    // Change the current task to the next task.
    //
    // We cannot directly drop `current` at this point. Since we are running as
    // `current`, we must avoid dropping `current`. Otherwise, the kernel stack
    // may be unmapped, leading to instant failure.
    let old_prev = PREVIOUS_TASK_PTR.load();
    PREVIOUS_TASK_PTR.store(current_task_ptr);
    CURRENT_TASK_PTR.store(Arc::into_raw(next_task));
    // Drop the old-previously running task.
    if !old_prev.is_null() {
        // SAFETY: The pointer is set by `switch_to_task` and is guaranteed to be
        // built with `Arc::into_raw`.
        drop(unsafe { Arc::from_raw(old_prev) });
    }

    drop(irq_guard);

    // SAFETY:
    // 1. `ctx` is only used in `reschedule()`. We have exclusive access to both the current task
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

static PREEMPT_COUNT: PreemptCount = PreemptCount::new();

struct PreemptCount {}

impl PreemptCount {
    const SHIFT: u8 = 0;

    const BITS: u8 = 31;

    const MASK: u32 = ((1 << Self::BITS) - 1) << Self::SHIFT;

    const fn new() -> Self {
        Self {}
    }

    fn inc(&self) {
        PREEMPT_INFO.add_assign(1 << Self::SHIFT);
    }

    fn dec(&self) {
        PREEMPT_INFO.sub_assign(1 << Self::SHIFT);
    }

    fn get(&self) -> u32 {
        PREEMPT_INFO.load() & Self::MASK
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
        PREEMPT_COUNT.inc();
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
        PREEMPT_COUNT.dec();
    }
}

/// Disables preemption.
pub fn disable_preempt() -> DisablePreemptGuard {
    DisablePreemptGuard::new()
}
