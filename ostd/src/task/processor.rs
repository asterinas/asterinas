// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{ptr::NonNull, sync::atomic::Ordering};

use super::{
    context_switch, disable_preempt, Task, TaskContext, POST_SCHEDULE_HANDLER, PRE_SCHEDULE_HANDLER,
};
use crate::{cpu::PinCurrentCpu, cpu_local_cell, trap::irq::DisabledLocalIrqGuard};

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

/// Returns a pointer to the current task running on the processor.
///
/// It returns `None` if the function is called in the bootstrap context.
pub(super) fn current_task() -> Option<NonNull<Task>> {
    NonNull::new(CURRENT_TASK_PTR.load().cast_mut())
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
#[track_caller]
pub(super) fn switch_to_task(next_task: Arc<Task>) {
    super::atomic_mode::might_sleep();

    // SAFETY: RCU read-side critical sections disables preemption. By the time
    // we reach this point, we have already checked that preemption is enabled.
    unsafe {
        crate::sync::finish_grace_period();
    }

    let irq_guard = crate::trap::irq::disable_local();

    let current_task_ptr = CURRENT_TASK_PTR.load();
    let current_task_ctx_ptr = if !current_task_ptr.is_null() {
        // SAFETY: The pointer is set by `switch_to_task` and is guaranteed to be
        // built with `Arc::into_raw`. It will only be dropped as a previous task,
        // so its reference will be valid until `after_switching_to`.
        let current_task = unsafe { &*current_task_ptr };

        // Until `after_switching_to`, the task's context is alive and can be exclusively used.
        current_task.ctx.get()
    } else {
        // Until `after_switching_to`, IRQs are disabled and the context can be exclusively used.
        BOOTSTRAP_CONTEXT.as_mut_ptr()
    };

    before_switching_to(&next_task, &irq_guard);

    // `before_switching_to` guarantees that from now on, and while the next task is running on the
    // CPU, its context can be used exclusively.
    let next_task_ctx_ptr = next_task.ctx().get().cast_const();

    CURRENT_TASK_PTR.store(Arc::into_raw(next_task));
    debug_assert!(PREVIOUS_TASK_PTR.load().is_null());
    PREVIOUS_TASK_PTR.store(current_task_ptr);

    // We must disable IRQs when switching, see `after_switching_to`.
    core::mem::forget(irq_guard);

    // SAFETY:
    // 1. We have exclusive access to both the current context and the next context (see above).
    // 2. The next context is valid (because it is either correctly initialized or written by a
    //    previous `context_switch`).
    unsafe {
        // This function may not return, for example, when the current task exits. So make sure
        // that all variables on the stack can be forgotten without causing resource leakage.
        context_switch(current_task_ctx_ptr, next_task_ctx_ptr);
    }

    // SAFETY: The task is just switched back, `after_switching_to` hasn't been called yet.
    unsafe { after_switching_to() };
}

fn before_switching_to(next_task: &Task, irq_guard: &DisabledLocalIrqGuard) {
    if let Some(handler) = PRE_SCHEDULE_HANDLER.get() {
        handler();
    }

    // Ensure that the mapping to the kernel stack is valid.
    next_task.kstack.flush_tlb(irq_guard);

    // Ensure that we are not switching to a task that is already running.
    while next_task
        .switched_to_cpu
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        log::warn!("Switching to a task already running in the foreground");
        core::hint::spin_loop();
    }
}

/// Does cleanups after switching to a task.
///
/// # Safety
///
/// This function must be called only once after switching to a task.
pub(super) unsafe fn after_switching_to() {
    // Release the previous task.
    let prev = PREVIOUS_TASK_PTR.load();
    let prev = if !prev.is_null() {
        PREVIOUS_TASK_PTR.store(core::ptr::null());

        // SAFETY: The pointer is set by `switch_to_task` and is guaranteed to
        // be built with `Arc::into_raw`. We couldn't do it twice since we set
        // it to NULL after the read.
        let prev_task = unsafe { Arc::from_raw(prev) };

        // Allows it to be switched on a CPU again, if anyone wants to.
        prev_task.switched_to_cpu.store(false, Ordering::Release);

        Some(prev_task)
    } else {
        None
    };

    let activated_anew = if let Some(handler) = POST_SCHEDULE_HANDLER.get() {
        handler()
    } else {
        true
    };

    let preempt_guard = disable_preempt();

    // See `switch_to_task`, where we forgot an IRQ guard.
    crate::arch::irq::enable_local();

    #[cfg(feature = "lazy_tlb_flush_on_unmap")]
    if let Some(cur_task) = Task::current() {
        if !activated_anew {
            let cur_cpu = preempt_guard.current_cpu();
            let prev_cpu = cur_task.prev_cpu.load(Ordering::Relaxed);
            cur_task
                .prev_cpu
                .store(cur_cpu.as_usize() as u32, Ordering::Relaxed);
            if prev_cpu != u32::MAX && prev_cpu != cur_cpu.as_usize() as u32 {
                // We are migrated here.
                crate::mm::tlb::latr::do_flush();
            }
        }
    }

    drop(preempt_guard);

    // It was forgotten using `Arc::into_raw` at `switch_to_task`.
    // We drop it after enabling the IRQ in case dropping user-provided
    // resources would violate the atomic mode.
    drop(prev);
}
