// SPDX-License-Identifier: MPL-2.0

use ostd::{
    task::{preempt, Task, TaskOptions},
    user::{ReturnReason, UserContextApi, UserMode, UserSpace},
};

use super::Thread;
use crate::{
    cpu::LinuxAbi,
    prelude::*,
    process::{posix_thread::PosixThreadExt, signal::handle_pending_signal},
    syscall::handle_syscall,
    thread::exception::handle_exception,
};

/// create new task with userspace and parent process
pub fn create_new_user_task(user_space: Arc<UserSpace>, thread_ref: Weak<Thread>) -> Arc<Task> {
    fn user_task_entry() {
        let current_thread = current_thread!();
        let current_task = current_thread.task();
        let user_space = current_task
            .user_space()
            .expect("user task should have user space");
        let mut user_mode = UserMode::new(user_space);
        debug!(
            "[Task entry] rip = 0x{:x}",
            user_mode.context().instruction_pointer()
        );
        debug!(
            "[Task entry] rsp = 0x{:x}",
            user_mode.context().stack_pointer()
        );
        debug!(
            "[Task entry] rax = 0x{:x}",
            user_mode.context().syscall_ret()
        );

        let posix_thread = current_thread.as_posix_thread().unwrap();
        let has_kernel_event_fn = || posix_thread.has_pending();
        loop {
            let return_reason = user_mode.execute(has_kernel_event_fn);
            let context = user_mode.context_mut();
            // handle user event:
            match return_reason {
                ReturnReason::UserException => handle_exception(context),
                ReturnReason::UserSyscall => handle_syscall(context),
                ReturnReason::KernelEvent => {}
            };

            if current_thread.status().is_exited() {
                break;
            }
            handle_pending_signal(context, &current_thread).unwrap();
            // If current is suspended, wait for a signal to wake up self
            while current_thread.status().is_stopped() {
                Thread::yield_now();
                debug!("{} is suspended.", current_thread.tid());
                handle_pending_signal(context, &current_thread).unwrap();
            }
            if current_thread.status().is_exited() {
                debug!("exit due to signal");
                break;
            }
            // a preemption point after handling user event.
            preempt(current_task);
        }
        debug!("exit user loop");
    }

    TaskOptions::new(user_task_entry)
        .data(thread_ref)
        .user_space(Some(user_space))
        .build()
        .expect("spawn task failed")
}
