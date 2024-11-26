// SPDX-License-Identifier: MPL-2.0

use ostd::{
    task::{Task, TaskOptions},
    user::{ReturnReason, UserContextApi, UserMode, UserSpace},
};

use super::{oops, Thread};
use crate::{
    cpu::LinuxAbi,
    current_userspace,
    prelude::*,
    process::{posix_thread::AsPosixThread, signal::handle_pending_signal},
    syscall::handle_syscall,
    thread::exception::handle_exception,
    vm::vmar::is_userspace_vaddr,
};

/// create new task with userspace and parent process
pub fn create_new_user_task(user_space: Arc<UserSpace>, thread_ref: Arc<Thread>) -> Task {
    fn user_task_entry() {
        let current_thread = current_thread!();
        let current_posix_thread = current_thread.as_posix_thread().unwrap();
        let current_process = current_posix_thread.process();
        let current_task = Task::current().unwrap();

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

        let child_tid_ptr = *current_posix_thread.set_child_tid().lock();

        // The `clone` syscall may require child process to write the thread pid to the specified address.
        // Make sure the store operation completes before the clone call returns control to user space
        // in the child process.
        if is_userspace_vaddr(child_tid_ptr) {
            current_userspace!()
                .write_val(child_tid_ptr, &current_posix_thread.tid())
                .unwrap();
        }

        let has_kernel_event_fn = || current_posix_thread.has_pending();

        let ctx = Context {
            process: current_process.as_ref(),
            posix_thread: current_posix_thread,
            thread: current_thread.as_ref(),
            task: current_task.as_ref(),
        };

        loop {
            let return_reason = user_mode.execute(has_kernel_event_fn);
            let user_ctx = user_mode.context_mut();
            // handle user event:
            match return_reason {
                ReturnReason::UserException => handle_exception(&ctx, user_ctx),
                ReturnReason::UserSyscall => handle_syscall(&ctx, user_ctx),
                ReturnReason::KernelEvent => {}
            };

            if current_thread.is_exited() {
                break;
            }
            handle_pending_signal(user_ctx, &ctx).unwrap();
            // If current is suspended, wait for a signal to wake up self
            while current_thread.is_stopped() {
                Thread::yield_now();
                debug!("{} is suspended.", current_posix_thread.tid());
                handle_pending_signal(user_ctx, &ctx).unwrap();
            }
            if current_thread.is_exited() {
                debug!("exit due to signal");
                break;
            }
        }
        debug!("exit user loop");
    }

    TaskOptions::new(|| {
        // TODO: If a kernel "oops" is caught, we should kill the entire
        // process rather than just ending the thread.
        let _ = oops::catch_panics_as_oops(user_task_entry);
    })
    .data(thread_ref)
    .user_space(Some(user_space))
    .build()
    .expect("spawn task failed")
}
