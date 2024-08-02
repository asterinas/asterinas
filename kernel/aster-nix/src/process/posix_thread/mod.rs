// SPDX-License-Identifier: MPL-2.0

//! POSIX thread information.
//!
//! A POSIX thread is a special kind of thread (defined in [`crate::thread`])
//! that has POSIX-specific data. So it is also a special [`ostd::task::Task`].

#![allow(dead_code)]

use aster_rights::{ReadOp, WriteOp};
use ostd::task::Task;

use super::{
    kill::SignalSenderIds,
    signal::{
        sig_mask::{AtomicSigMask, SigSet},
        sig_num::SigNum,
        sig_queues::SigQueues,
        signals::Signal,
        SigEvents, SigEventsFilter, SigStack,
    },
    Credentials, Process,
};
use crate::{
    events::Observer,
    prelude::*,
    process::signal::constants::SIGCONT,
    thread::Tid,
    time::{clocks::ProfClock, Timer, TimerManager},
};

mod builder;
mod exit;
pub mod futex;
mod name;
mod robust_list;

pub use builder::PosixThreadBuilder;
pub use exit::do_exit;
pub use name::{ThreadName, MAX_THREAD_NAME_LEN};
pub use robust_list::RobustListHead;

/// Extra operations that can be operated on tasks that are POSIX threads.
pub trait PosixThreadExt {
    fn posix_thread_info(&self) -> Option<&SharedPosixThreadInfo>;
    fn posix_thread_info_mut(&mut self) -> Option<&mut MutPosixThreadInfo>;
}

impl PosixThreadExt for Task {
    fn posix_thread_info(&self) -> Option<&SharedPosixThreadInfo> {
        self.shared_data().downcast_ref::<SharedPosixThreadInfo>()
    }

    fn posix_thread_info_mut(&mut self) -> Option<&mut MutPosixThreadInfo> {
        self.mut_data().downcast_mut::<MutPosixThreadInfo>()
    }
}

pub struct SharedPosixThreadInfo {
    // The process that the thread belongs to.
    pub process: Weak<Process>,

    // Thread name.
    pub name: RwLock<Option<ThreadName>>,

    // Linux specific attributes.
    // https://man7.org/linux/man-pages/man2/set_tid_address.2.html
    pub set_child_tid: RwLock<Vaddr>,
    pub clear_child_tid: RwLock<Vaddr>,

    pub robust_list: Mutex<Option<RobustListHead>>,

    /// Process credentials. At the kernel level, credentials are a per-thread attribute.
    pub credentials: Credentials,

    /// Blocked signals
    pub sig_mask: AtomicSigMask,

    /// Thread-directed signal queues.
    pub sig_queues: SigQueues,

    /// A profiling clock measures the user CPU time and kernel CPU time in the thread.
    pub prof_clock: Arc<ProfClock>,

    /// A manager that manages timers based on the user CPU time of the current thread.
    pub virtual_timer_manager: Arc<TimerManager>,

    /// A manager that manages timers based on the profiling clock of the current thread.
    pub prof_timer_manager: Arc<TimerManager>,
}

pub struct MutPosixThreadInfo {
    /// Signal handler ucontext address
    /// FIXME: This field may be removed. For glibc applications with RESTORER flag set, the sig_context is always equals with rsp.
    pub sig_context: Option<Vaddr>,
    pub sig_stack: Option<SigStack>,
}

impl SharedPosixThreadInfo {
    pub fn process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }

    /// Checks whether the signal can be delivered to the thread.
    ///
    /// For a signal can be delivered to the thread, the sending thread must either
    /// be privileged, or the real or effective user ID of the sending thread must equal
    /// the real or saved set-user-ID of the target thread.
    ///
    /// For SIGCONT, the sending and receiving processes should belong to the same session.
    pub(in crate::process) fn check_signal_perm(
        &self,
        signum: Option<&SigNum>,
        sender: &SignalSenderIds,
    ) -> Result<()> {
        if sender.euid().is_root() {
            return Ok(());
        }

        if let Some(signum) = signum
            && *signum == SIGCONT
        {
            let receiver_sid = self.process().session().unwrap().sid();
            if receiver_sid == sender.sid() {
                return Ok(());
            }

            return_errno_with_message!(
                Errno::EPERM,
                "sigcont requires that sender and receiver belongs to the same session"
            );
        }

        let (receiver_ruid, receiver_suid) = {
            let credentials = self.credentials();
            (credentials.ruid(), credentials.suid())
        };

        // FIXME: further check the below code to ensure the behavior is same as Linux. According
        // to man(2) kill, the real or effective user ID of the sending process must equal the
        // real or saved set-user-ID of the target process.
        if sender.ruid() == receiver_ruid
            || sender.ruid() == receiver_suid
            || sender.euid() == receiver_ruid
            || sender.euid() == receiver_suid
        {
            return Ok(());
        }

        return_errno_with_message!(Errno::EPERM, "sending signal to the thread is not allowed.");
    }

    /// Gets the read-only credentials of the thread.
    pub fn credentials(&self) -> Credentials<ReadOp> {
        self.credentials.dup().restrict()
    }

    /// Gets the write-only credentials of the thread.
    pub(in crate::process) fn credentials_mut(&self) -> Credentials<WriteOp> {
        self.credentials.dup().restrict()
    }
}
