// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::context::UserContext,
    task::{Task, TaskOptions},
    user::{ReturnReason, UserContextApi, UserMode},
};

use super::{oops, Thread};
use crate::{
    cpu::LinuxAbi,
    current_userspace,
    prelude::*,
    process::{
        posix_thread::{AsPosixThread, AsThreadLocal, ThreadLocal},
        signal::handle_pending_signal,
    },
    syscall::handle_syscall,
    thread::{exception::handle_exception, AsThread},
    vm::vmar::is_userspace_vaddr,
};

/// create new task with userspace and parent process
pub fn create_new_user_task(
    user_ctx: Arc<UserContext>,
    thread_ref: Arc<Thread>,
    thread_local: ThreadLocal,
) -> Task {
    fn user_task_entry() {
        let current_task = Task::current().unwrap();
        let current_thread = current_task.as_thread().unwrap();
        let current_posix_thread = current_thread.as_posix_thread().unwrap();
        let current_thread_local = current_task.as_thread_local().unwrap();
        let current_process = current_posix_thread.process();

        let user_ctx = current_task
            .user_ctx()
            .expect("user task should have user context");
        let mut user_mode = UserMode::new(UserContext::clone(user_ctx));
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

        let child_tid_ptr = current_thread_local.set_child_tid().get();

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
            thread_local: current_thread_local,
            posix_thread: current_posix_thread,
            thread: current_thread.as_ref(),
            task: &current_task,
        };

        loop {
            let return_reason = user_mode.execute(has_kernel_event_fn);
            let user_ctx = user_mode.context_mut();
            let mut syscall_number = None;
            // handle user event:
            match return_reason {
                ReturnReason::UserException => handle_exception(&ctx, user_ctx),
                ReturnReason::UserSyscall => {
                    syscall_number = Some(user_ctx.syscall_num());
                    handle_syscall(&ctx, user_ctx);
                }
                ReturnReason::KernelEvent => {}
            };

            if current_thread.is_exited() {
                break;
            }
            handle_pending_signal(user_ctx, &ctx, syscall_number);
            // If current is suspended, wait for a signal to wake up self
            while current_thread.is_stopped() {
                Thread::yield_now();
                debug!("{} is suspended.", current_posix_thread.tid());
                handle_pending_signal(user_ctx, &ctx, None);
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
    .local_data(thread_local)
    .user_ctx(Some(user_ctx))
    .build()
    .expect("spawn task failed")
}
