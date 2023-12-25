use aster_frame::{
    cpu::UserContext,
    task::{preempt, Task, TaskOptions},
    user::{UserContextApi, UserEvent, UserMode, UserSpace},
};

use crate::{
    prelude::*, process::signal::handle_pending_signal, syscall::handle_syscall,
    thread::exception::handle_exception,
};

use super::Thread;

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

    TaskOptions::new(user_task_entry)
        .data(thread_ref)
        .user_space(Some(user_space))
        .build()
        .expect("spawn task failed")
}

fn handle_user_event(user_event: UserEvent, context: &mut UserContext) {
    match user_event {
        UserEvent::Syscall => handle_syscall(context),
        UserEvent::Exception => handle_exception(context),
    }
}
