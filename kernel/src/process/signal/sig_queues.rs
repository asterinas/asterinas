// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use super::{
    constants::*,
    sig_mask::{SigMask, SigSet},
    sig_num::SigNum,
    signals::Signal,
    SigEvents, SigEventsFilter,
};
use crate::{
    events::{Observer, Subject},
    prelude::*,
};

pub struct SigQueues {
    // The number of pending signals.
    // Useful for quickly determining if any signals are pending without locking `queues`.
    count: AtomicUsize,
    queues: Mutex<Queues>,
    subject: Subject<SigEvents, SigEventsFilter>,
}

impl SigQueues {
    pub fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            queues: Mutex::new(Queues::new()),
            subject: Subject::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0
    }

    pub fn enqueue(&self, signal: Box<dyn Signal>) {
        let signum = signal.num();

        let mut queues = self.queues.lock();
        if queues.enqueue(signal) {
            self.count.fetch_add(1, Ordering::Relaxed);
            // Avoid holding lock when notifying observers
            drop(queues);
            self.subject.notify_observers(&SigEvents::new(signum));
        }
    }

    pub fn dequeue(&self, blocked: &SigMask) -> Option<Box<dyn Signal>> {
        // Fast path for the common case of no pending signals
        if self.is_empty() {
            return None;
        }

        let mut queues = self.queues.lock();
        let signal = queues.dequeue(blocked);
        if signal.is_some() {
            self.count.fetch_sub(1, Ordering::Relaxed);
        }
        signal
    }

    /// Returns the pending signals
    pub fn sig_pending(&self) -> SigSet {
        let queues = self.queues.lock();
        queues.sig_pending()
    }

    /// Returns whether there's some pending signals that are not blocked
    pub fn has_pending(&self, blocked: SigMask) -> bool {
        if self.is_empty() {
            return false;
        }
        self.queues.lock().has_pending(blocked)
    }

    pub fn register_observer(
        &self,
        observer: Weak<dyn Observer<SigEvents>>,
        filter: SigEventsFilter,
    ) {
        self.subject.register_observer(observer, filter);
    }

    pub fn unregister_observer(&self, observer: &Weak<dyn Observer<SigEvents>>) {
        self.subject.unregister_observer(observer);
    }
}

impl Default for SigQueues {
    fn default() -> Self {
        Self::new()
    }
}

struct Queues {
    std_queues: Vec<Option<Box<dyn Signal>>>,
    rt_queues: Vec<VecDeque<Box<dyn Signal>>>,
}

impl Queues {
    fn new() -> Self {
        let std_queues = (0..COUNT_STD_SIGS).map(|_| None).collect();
        let rt_queues = (0..COUNT_RT_SIGS).map(|_| Default::default()).collect();
        Self {
            std_queues,
            rt_queues,
        }
    }

    fn enqueue(&mut self, signal: Box<dyn Signal>) -> bool {
        let signum = signal.num();
        if signum.is_std() {
            // Standard signals
            //
            // From signal(7):
            //
            // Standard signals do not queue.  If multiple instances of a standard
            // signal are generated while that signal is blocked, then only one
            // instance of the signal is marked as pending (and the signal will be
            // delivered just once when it is unblocked).  In the case where a
            // standard signal is already pending, the siginfo_t structure (see
            // sigaction(2)) associated with that signal is not overwritten on
            // arrival of subsequent instances of the same signal.  Thus, the
            // process will receive the information associated with the first
            // instance of the signal.
            let queue = self.get_std_queue_mut(signum);
            if queue.is_some() {
                // If there is already a signal pending, just ignore all subsequent signals
                return false;
            }
            *queue = Some(signal);
        } else {
            // Real-time signals
            let queue = self.get_rt_queue_mut(signum);
            queue.push_back(signal);
        }

        true
    }

    fn dequeue(&mut self, blocked: &SigMask) -> Option<Box<dyn Signal>> {
        // Deliver standard signals.
        //
        // According to signal(7):
        // If both standard and real-time signals are pending for a process,
        // POSIX leaves it unspecified which is delivered first. Linux, like
        // many other implementations, gives priority to standard signals in
        // this case.

        // POSIX leaves unspecified which to deliver first if there are multiple
        // pending standard signals. So we are free to define our own. The
        // principle is to give more urgent signals higher priority (like SIGKILL).

        // FIXME: the gvisor pty_test JobControlTest::ReleaseTTY requires that
        // the SIGHUP signal should be handled before SIGCONT.
        const ORDERED_STD_SIGS: [SigNum; COUNT_STD_SIGS] = [
            SIGKILL, SIGTERM, SIGSTOP, SIGSEGV, SIGILL, SIGHUP, SIGCONT, SIGINT, SIGQUIT, SIGTRAP,
            SIGABRT, SIGBUS, SIGFPE, SIGUSR1, SIGUSR2, SIGPIPE, SIGALRM, SIGSTKFLT, SIGCHLD,
            SIGTSTP, SIGTTIN, SIGTTOU, SIGURG, SIGXCPU, SIGXFSZ, SIGVTALRM, SIGPROF, SIGWINCH,
            SIGIO, SIGPWR, SIGSYS,
        ];
        for &signum in &ORDERED_STD_SIGS {
            if blocked.contains(signum) {
                continue;
            }

            let queue = self.get_std_queue_mut(signum);
            let signal = queue.take();
            if signal.is_some() {
                return signal;
            }
        }

        // If no standard signals, then deliver real-time signals.
        //
        // According to signal (7):
        // Real-time signals are delivered in a guaranteed order.  Multiple
        // real-time signals of the same type are delivered in the order
        // they were sent.  If different real-time signals are sent to a
        // process, they are delivered starting with the lowest-numbered
        // signal.  (I.e., low-numbered signals have highest priority.)
        for signum in MIN_RT_SIG_NUM..=MAX_RT_SIG_NUM {
            let signum = SigNum::try_from(signum).unwrap();
            if blocked.contains(signum) {
                continue;
            }

            let queue = self.get_rt_queue_mut(signum);
            let signal = queue.pop_front();
            if signal.is_some() {
                return signal;
            }
        }

        // There must be pending but blocked signals
        None
    }

    /// Returns whether the `SigQueues` has some pending signals which are not blocked
    fn has_pending(&self, blocked: SigMask) -> bool {
        self.std_queues.iter().any(|signal| {
            signal
                .as_ref()
                .is_some_and(|signal| !blocked.contains(signal.num()))
        }) || self.rt_queues.iter().any(|rt_queue| !rt_queue.is_empty())
    }

    fn get_std_queue_mut(&mut self, signum: SigNum) -> &mut Option<Box<dyn Signal>> {
        debug_assert!(signum.is_std());
        let idx = (signum.as_u8() - MIN_STD_SIG_NUM) as usize;
        &mut self.std_queues[idx]
    }

    fn get_rt_queue_mut(&mut self, signum: SigNum) -> &mut VecDeque<Box<dyn Signal>> {
        debug_assert!(signum.is_real_time());
        let idx = (signum.as_u8() - MIN_RT_SIG_NUM) as usize;
        &mut self.rt_queues[idx]
    }

    fn sig_pending(&self) -> SigSet {
        let mut pending = SigSet::new_empty();

        // Process standard signal queues
        for (idx, signal) in self.std_queues.iter().enumerate() {
            if signal.is_some() {
                pending += SigNum::from_u8(idx as u8 + MIN_STD_SIG_NUM);
            }
        }

        // Process real-time signal queues
        for (idx, signals) in self.rt_queues.iter().enumerate() {
            if !signals.is_empty() {
                pending += SigNum::from_u8(idx as u8 + MIN_RT_SIG_NUM);
            }
        }

        pending
    }
}
