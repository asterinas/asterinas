// SPDX-License-Identifier: MPL-2.0

//! Thread information.
//!
//! A thread is a special kind of [`Task`] that has thread-specific data.

pub mod exception;
mod kernel_thread;
pub mod status;
pub mod thread_table;
pub mod work_queue;

use core::sync::atomic::{AtomicU32, Ordering};

pub use kernel_thread::{
    new_kernel, KernelThreadContext, MutKernelThreadInfo, SharedKernelThreadInfo,
};
use ostd::{
    cpu::UserContext,
    task::{MutTaskInfo, SharedTaskInfo, Task, TaskOptions},
    user::{ReturnReason, UserContextApi, UserMode, UserSpace},
};
use status::{AtomicThreadStatus, ThreadStatus};

use crate::{
    cpu::LinuxAbi,
    prelude::*,
    process::{
        do_exit_group,
        posix_thread::{MutPosixThreadInfo, PosixThreadContext, SharedPosixThreadInfo},
        signal::{
            handle_user_signal,
            sig_action::{SigAction, SigDefaultAction},
        },
        TermStatus,
    },
    syscall::handle_syscall,
    thread::exception::handle_exception,
};

pub type Tid = u32;

static TID_ALLOCATOR: AtomicU32 = AtomicU32::new(0);

/// The part of the thread data that is shared between all threads.
pub struct SharedThreadInfo {
    pub tid: Tid,
    pub status: AtomicThreadStatus,
}

/// The part of the thread data that is exclusive to the current thread.
pub struct MutThreadInfo {
    // To prevent the struct from being instantiated from outside the module.
    _private: (),
}

/// Extra operations that can be operated on thread tasks.
pub trait ThreadExt {
    fn thread_info(&self) -> Option<&SharedThreadInfo>;
    fn mut_thread_info(&mut self) -> Option<&mut MutThreadInfo>;
}

impl ThreadExt for Task {
    fn thread_info(&self) -> Option<&SharedThreadInfo> {
        self.shared_data().downcast_ref::<SharedThreadInfo>()
    }

    fn mut_thread_info(&mut self) -> Option<&mut MutThreadInfo> {
        self.mut_data().downcast_mut::<MutThreadInfo>()
    }
}

impl MutThreadInfo {
    pub(super) fn exit(&mut self, info: &SharedThreadInfo) {
        info.status.store(ThreadStatus::Exited, Ordering::Release);
    }

    pub fn join(&mut self, task_ctx_mut: &mut MutTaskInfo, thread: Arc<Task>) {
        loop {
            let status = thread.thread_info().unwrap().status.load(Ordering::Acquire);
            if status.is_exited() {
                break;
            } else {
                task_ctx_mut.yield_now();
            }
        }
    }
}

/// Allocates a new tid for the new thread
pub fn allocate_tid() -> Tid {
    TID_ALLOCATOR.fetch_add(1, Ordering::SeqCst)
}

/// Creates a new user thread with user space and parent process.
pub fn new_user(
    user_space: Arc<UserSpace>,
    tid: Tid,
    mut_pthread_info: MutPosixThreadInfo,
    shared_pthread_info: SharedPosixThreadInfo,
) -> Arc<Task> {
    fn user_task_entry(
        task_ctx_mut: &mut MutTaskInfo,
        task_ctx: &SharedTaskInfo,
        task_data_mut: &mut dyn Any,
        task_data: &(dyn Any + Send + Sync),
    ) {
        let thread_data_mut = task_data_mut.downcast_mut::<MutThreadTaskData>().unwrap();
        let thread_data = task_data.downcast_ref::<ShareThreadTaskData>().unwrap();

        let user_space = task_ctx
            .user_space
            .as_ref()
            .expect("User task should have user space");
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

        let thread_ctx_mut = &mut thread_data_mut.info;
        let thread_ctx = &thread_data.info;

        let pthread_ctx_mut = thread_data_mut
            .ext_data
            .downcast_mut::<MutPosixThreadInfo>()
            .unwrap();
        let pthread_ctx = thread_data
            .ext_data
            .downcast_ref::<SharedPosixThreadInfo>()
            .unwrap();

        loop {
            let return_reason = user_mode.execute();
            let context = user_mode.context_mut();
            // handle user event:
            match return_reason {
                ReturnReason::UserException => handle_exception(context, pthread_ctx),
                ReturnReason::UserSyscall => handle_syscall(
                    context,
                    task_ctx_mut,
                    task_ctx,
                    thread_ctx_mut,
                    thread_ctx,
                    pthread_ctx_mut,
                    pthread_ctx,
                ),
                ReturnReason::Interrupt => {}
            };

            if thread_ctx.status.load(Ordering::Acquire).is_exited() {
                break;
            }
            handle_pending_signal(context, thread_ctx, pthread_ctx_mut, pthread_ctx).unwrap();
            // If current is suspended, wait for a signal to wake up self
            while thread_ctx.status.load(Ordering::Acquire).is_stopped() {
                task_ctx_mut.yield_now();
                debug!("{} is suspended.", thread_ctx.tid);
                handle_pending_signal(context, thread_ctx, pthread_ctx_mut, pthread_ctx).unwrap();
            }
            if thread_ctx.status.load(Ordering::Acquire).is_exited() {
                debug!("exit due to signal");
                break;
            }
            // a preemption point after handling user event.
            task_ctx_mut.preempt(task_ctx);
        }
        debug!("exit user loop");
    }

    let mut_data = MutThreadTaskData {
        info: MutThreadInfo { _private: () },
        ext_data: Box::new(mut_pthread_info),
    };

    let shared_data = ShareThreadTaskData {
        info: SharedThreadInfo {
            tid,
            status: AtomicThreadStatus::new(ThreadStatus::Init),
        },
        ext_data: Box::new(shared_pthread_info),
    };

    let thread = TaskOptions::new(user_task_entry)
        .shared_data(shared_data)
        .mut_data(mut_data)
        .user_space(Some(user_space))
        .build()
        .expect("Failed to build a user task");

    thread_table::add_thread(thread.clone());

    thread
}

/// Handle pending signal for current process.
fn handle_pending_signal(
    context: &mut UserContext,
    thread_ctx: &SharedThreadInfo,
    pthread_ctx_mut: &mut MutPosixThreadInfo,
    pthread_ctx: &SharedPosixThreadInfo,
) -> Result<()> {
    // We first deal with signal in current thread, then signal in current process.
    let signal = {
        let sig_mask = pthread_ctx.sig_mask.load(Ordering::Acquire);
        if let Some(signal) = pthread_ctx.sig_queues.dequeue(&sig_mask) {
            signal
        } else {
            return Ok(());
        }
    };

    let sig_num = signal.num();
    trace!("sig_num = {:?}, sig_name = {}", sig_num, sig_num.sig_name());
    let current = pthread_ctx.process();
    let sig_action = current.sig_dispositions().lock().get(sig_num);
    trace!("sig action: {:x?}", sig_action);
    match sig_action {
        SigAction::Ign => {
            trace!("Ignore signal {:?}", sig_num);
        }
        SigAction::User {
            handler_addr,
            flags,
            restorer_addr,
            mask,
        } => handle_user_signal(
            context,
            pthread_ctx_mut,
            pthread_ctx,
            sig_num,
            handler_addr,
            flags,
            restorer_addr,
            mask,
            signal.to_info(),
        )?,
        SigAction::Dfl => {
            let sig_default_action = SigDefaultAction::from_signum(sig_num);
            trace!("sig_default_action: {:?}", sig_default_action);
            match sig_default_action {
                SigDefaultAction::Core | SigDefaultAction::Term => {
                    warn!(
                        "{:?}: terminating on signal {}",
                        current.executable_path(),
                        sig_num.sig_name()
                    );
                    // We should exit current here, since we cannot restore a valid status from trap now.
                    do_exit_group(TermStatus::Killed(sig_num));
                }
                SigDefaultAction::Ign => {}
                SigDefaultAction::Stop => {
                    let _ = thread_ctx.status.compare_exchange(
                        ThreadStatus::Running,
                        ThreadStatus::Stopped,
                        Ordering::AcqRel,
                        Ordering::Relaxed,
                    );
                }
                SigDefaultAction::Cont => {
                    let _ = thread_ctx.status.compare_exchange(
                        ThreadStatus::Stopped,
                        ThreadStatus::Running,
                        Ordering::AcqRel,
                        Ordering::Relaxed,
                    );
                }
            }
        }
    }
    Ok(())
}

struct ShareThreadTaskData {
    info: SharedThreadInfo,
    // Extra shared data for the thread. The type of the extra data defines
    // the type of the thread. A thread can be either a kernel thread or a
    // POSIX thread.
    ext_data: Box<dyn Any + Send + Sync>,
}

struct MutThreadTaskData {
    info: MutThreadInfo,
    // Extra mutable data for the thread. The type of the extra data defines
    // the type of the thread. A thread can be either a kernel thread or a
    // POSIX thread.
    ext_data: Box<dyn Any>,
}
