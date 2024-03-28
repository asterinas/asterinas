// SPDX-License-Identifier: MPL-2.0

use aster_frame::{
    cpu::{CpuSet, UserContext},
    task::{preempt, Task, TaskOptions},
    user::{UserContextApi, UserEvent, UserMode, UserSpace},
};

use super::Thread;
use crate::{
    prelude::*, process::signal::handle_pending_signal, syscall::handle_syscall,
    thread::exception::handle_exception,
};

/// create new task with userspace and parent process
pub fn create_new_user_task(user_space: Arc<UserSpace>, thread_ref: Weak<Thread>) -> Arc<Task> {
    fn user_task_entry() {
        let cur = Task::current();
        let user_space = cur.user_space().expect("user task should have user space");
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
        loop {
            let user_event = user_mode.execute();
            let context = user_mode.context_mut();
            // handle user event:
            handle_user_event(user_event, context);
            let current_thread = current_thread!();
            // should be do this comparison before handle signal?
            if current_thread.status().lock().is_exited() {
                break;
            }
            handle_pending_signal(context).unwrap();
            if current_thread.status().lock().is_exited() {
                debug!("exit due to signal");
                break;
            }
            // If current is suspended, wait for a signal to wake up self
            while current_thread.status().lock().is_stopped() {
                Thread::yield_now();
                debug!("{} is suspended.", current_thread.tid());
                handle_pending_signal(context).unwrap();
            }
            // a preemption point after handling user event.
            preempt();
        }
        debug!("exit user loop");
        // FIXME: This is a work around: exit in kernel task entry may be not called. Why this will happen?
        Task::current().exit();
    }
    // **FIXME**: All user tasks are currently being bound to a single core due to limitations in the
    // current Copy-On-Write (COW) mechanism. After a parent process clones a child process, if
    // the child process is switched to another core, and the parent process returns to user space,
    // it might modify the state of the user stack. This could lead to the child process accessing
    // invalid addresses. Binding to a single core avoids this issue because the parent process will
    // switch immediately to the child process upon clone, so the stack state remains unchanged temporarily.
    // This constraint should be revisited and the CPU affinity binding removed once a robust COW mechanism
    // is implemented.
    let mut cpu_set = CpuSet::new_empty();
    cpu_set.add(0);
    TaskOptions::new(user_task_entry)
        .data(thread_ref)
        .user_space(Some(user_space))
        .cpu_affinity(cpu_set)
        .build()
        .expect("spawn task failed")
}

fn handle_user_event(user_event: UserEvent, context: &mut UserContext) {
    match user_event {
        UserEvent::Syscall => handle_syscall(context),
        UserEvent::Exception => handle_exception(context),
    }
}
