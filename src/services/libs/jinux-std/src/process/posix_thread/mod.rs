use crate::{
    prelude::*,
    process::posix_thread::{futex::futex_wake, robust_list::wake_robust_futex},
    thread::{thread_table, Tid},
    util::write_val_to_user,
};

use self::{name::ThreadName, robust_list::RobustListHead};

use super::{
    signal::{sig_mask::SigMask, sig_queues::SigQueues},
    Process,
};

pub mod builder;
pub mod futex;
pub mod name;
pub mod posix_thread_ext;
pub mod robust_list;

pub struct PosixThread {
    // Immutable part
    process: Weak<Process>,
    is_main_thread: bool,

    // Mutable part
    name: Mutex<Option<ThreadName>>,

    // Linux specific attributes.
    // https://man7.org/linux/man-pages/man2/set_tid_address.2.html
    set_child_tid: Mutex<Vaddr>,
    clear_child_tid: Mutex<Vaddr>,

    robust_list: Mutex<Option<RobustListHead>>,

    // signal
    /// blocked signals
    sig_mask: Mutex<SigMask>,
    /// thread-directed sigqueue
    sig_queues: Mutex<SigQueues>,
    /// Signal handler ucontext address
    /// FIXME: This field may be removed. For glibc applications with RESTORER flag set, the sig_context is always equals with rsp.
    sig_context: Mutex<Option<Vaddr>>,
}

impl PosixThread {
    pub fn process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }

    pub fn thread_name(&self) -> &Mutex<Option<ThreadName>> {
        &self.name
    }

    pub fn set_child_tid(&self) -> &Mutex<Vaddr> {
        &self.set_child_tid
    }

    pub fn clear_child_tid(&self) -> &Mutex<Vaddr> {
        &self.clear_child_tid
    }

    pub fn sig_mask(&self) -> &Mutex<SigMask> {
        &self.sig_mask
    }

    pub fn sig_queues(&self) -> &Mutex<SigQueues> {
        &self.sig_queues
    }

    pub fn sig_context(&self) -> &Mutex<Option<Vaddr>> {
        &self.sig_context
    }

    pub fn robust_list(&self) -> &Mutex<Option<RobustListHead>> {
        &self.robust_list
    }

    /// Whether the thread is main thread. For Posix thread, If a thread's tid is equal to pid, it's main thread.
    pub fn is_main_thread(&self) -> bool {
        self.is_main_thread
    }

    /// whether the thread is the last running thread in process
    pub fn is_last_thread(&self) -> bool {
        let process = self.process.upgrade().unwrap();
        let threads = process.threads().lock();
        threads
            .iter()
            .filter(|thread| !thread.status().lock().is_exited())
            .count()
            == 0
    }

    /// Walks the robust futex list, marking futex dead and wake waiters.
    /// It corresponds to Linux's exit_robust_list(), errors are silently ignored.
    pub fn wake_robust_list(&self, tid: Tid) {
        let mut robust_list = self.robust_list.lock();
        let list_head = match *robust_list {
            None => {
                return;
            }
            Some(robust_list_head) => robust_list_head,
        };
        debug!("wake the rubust_list: {:?}", list_head);
        for futex_addr in list_head.futexes() {
            // debug!("futex addr = 0x{:x}", futex_addr);
            wake_robust_futex(futex_addr, tid).unwrap();
        }
        debug!("wake robust futex success");
        *robust_list = None;
    }

    /// Posix thread does not contains tid info. So we require tid as a parameter.
    pub fn exit(&self, tid: Tid, exit_code: i32) -> Result<()> {
        let mut clear_ctid = self.clear_child_tid().lock();
        // If clear_ctid !=0 ,do a futex wake and write zero to the clear_ctid addr.
        debug!("wake up ctid");
        if *clear_ctid != 0 {
            futex_wake(*clear_ctid, 1)?;
            // FIXME: the correct write length?
            write_val_to_user(*clear_ctid, &0i32)?;
            *clear_ctid = 0;
        }
        // exit the robust list: walk the robust list; mark futex words as dead and do futex wake
        self.wake_robust_list(tid);

        if tid != self.process().pid {
            // If the thread is not main thread. We don't remove main thread.
            // Main thread are removed when the whole process is reaped.
            thread_table::remove_thread(tid);
        }

        if self.is_main_thread() || self.is_last_thread() {
            // exit current process.
            debug!("self is main thread or last thread");
            debug!("main thread: {}", self.is_main_thread());
            debug!("last thread: {}", self.is_last_thread());
            current!().exit_group(exit_code);
        }
        debug!("perform futex wake");
        futex_wake(Arc::as_ptr(&self.process()) as Vaddr, 1)?;
        Ok(())
    }
}
