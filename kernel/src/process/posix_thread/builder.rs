// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use ostd::{cpu::CpuSet, task::Task, user::UserSpace};

use super::{thread_table, PosixThread};
use crate::{
    prelude::*,
    process::{
        posix_thread::name::ThreadName,
        signal::{sig_mask::AtomicSigMask, sig_queues::SigQueues},
        Credentials, Process,
    },
    sched::priority::Priority,
    thread::{status::ThreadStatus, task, Thread, Tid},
    time::{clocks::ProfClock, TimerManager},
};

/// The builder to build a posix thread
pub struct PosixThreadBuilder {
    // The essential part
    tid: Tid,
    user_space: Arc<UserSpace>,
    process: Weak<Process>,
    credentials: Credentials,

    // Optional part
    thread_name: Option<ThreadName>,
    set_child_tid: Vaddr,
    clear_child_tid: Vaddr,
    sig_mask: AtomicSigMask,
    sig_queues: SigQueues,
}

impl PosixThreadBuilder {
    pub fn new(tid: Tid, user_space: Arc<UserSpace>, credentials: Credentials) -> Self {
        Self {
            tid,
            user_space,
            process: Weak::new(),
            credentials,
            thread_name: None,
            set_child_tid: 0,
            clear_child_tid: 0,
            sig_mask: AtomicSigMask::new_empty(),
            sig_queues: SigQueues::new(),
        }
    }

    pub fn process(mut self, process: Weak<Process>) -> Self {
        self.process = process;
        self
    }

    pub fn thread_name(mut self, thread_name: Option<ThreadName>) -> Self {
        self.thread_name = thread_name;
        self
    }

    pub fn set_child_tid(mut self, set_child_tid: Vaddr) -> Self {
        self.set_child_tid = set_child_tid;
        self
    }

    pub fn clear_child_tid(mut self, clear_child_tid: Vaddr) -> Self {
        self.clear_child_tid = clear_child_tid;
        self
    }

    pub fn sig_mask(mut self, sig_mask: AtomicSigMask) -> Self {
        self.sig_mask = sig_mask;
        self
    }

    pub fn build(self) -> Arc<Task> {
        let Self {
            tid,
            user_space,
            process,
            credentials,
            thread_name,
            set_child_tid,
            clear_child_tid,
            sig_mask,
            sig_queues,
        } = self;

        Arc::new_cyclic(|weak_task| {
            let posix_thread = {
                let prof_clock = ProfClock::new();
                let virtual_timer_manager = TimerManager::new(prof_clock.user_clock().clone());
                let prof_timer_manager = TimerManager::new(prof_clock.clone());

                PosixThread {
                    process,
                    tid,
                    name: Mutex::new(thread_name),
                    set_child_tid: Mutex::new(set_child_tid),
                    clear_child_tid: Mutex::new(clear_child_tid),
                    credentials,
                    sig_mask,
                    sig_queues,
                    sig_context: Mutex::new(None),
                    sig_stack: Mutex::new(None),
                    signalled_waker: SpinLock::new(None),
                    robust_list: Mutex::new(None),
                    prof_clock,
                    virtual_timer_manager,
                    prof_timer_manager,
                }
            };

            let status = ThreadStatus::Init;
            let priority = Priority::default();
            let cpu_affinity = CpuSet::new_full();
            let thread = Arc::new(Thread::new(
                weak_task.clone(),
                posix_thread,
                status,
                priority,
                cpu_affinity,
            ));

            thread_table::add_thread(tid, thread.clone());
            task::create_new_user_task(user_space, thread)
        })
    }
}
