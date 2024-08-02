// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use ostd::user::UserSpace;

use super::{MutPosixThreadInfo, SharedPosixThreadInfo};
use crate::{
    prelude::*,
    process::{
        posix_thread::name::ThreadName,
        signal::{sig_mask::AtomicSigMask, sig_queues::SigQueues},
        Credentials, Process,
    },
    thread::{self, Tid},
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

        let shared_posix_thread_info = SharedPosixThreadInfo {
            process,
            name: RwLock::new(thread_name),
            set_child_tid: RwLock::new(set_child_tid),
            clear_child_tid: RwLock::new(clear_child_tid),
            robust_list: Mutex::new(None),
            credentials,
            sig_queues,
            sig_mask,
            prof_clock: ProfClock::new(),
            virtual_timer_manager: TimerManager::new(prof_clock.user_clock().clone()),
            prof_timer_manager: TimerManager::new(prof_clock.clone()),
        };

        let mutable_posix_thread_info = MutPosixThreadInfo {
            sig_context: None,
            sig_stack: None,
        };

        thread::new_user(
            user_space,
            tid,
            mutable_posix_thread_info,
            shared_posix_thread_info,
        )
    }
}
