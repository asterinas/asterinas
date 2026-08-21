// SPDX-License-Identifier: MPL-2.0

use super::kernel_thread::ThreadOptions;

pub(super) fn init_in_first_kthread() {
    // Spawn softirq daemons on each CPU.
    for cpu in ostd::cpu::all_cpus() {
        ThreadOptions::new(aster_softirq::daemon_loop_on_cpu)
            .cpu_affinity(cpu.into())
            .spawn();
    }
}

#[cfg(ktest)]
mod test {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use aster_softirq::SoftIrqLine;
    use ostd::{irq::InterruptLevel, prelude::ktest, sync::Waiter};

    const SOFTIRQ_ID: u8 = 7;

    const MAX_NUM_SOFTIRQS: usize = 256;

    /// The number of callbacks from the bottom half.
    static COUNTER_BH: AtomicUsize = AtomicUsize::new(0);

    /// The number of callbacks from the daemon thread.
    static COUNTER_DAEMON: AtomicUsize = AtomicUsize::new(0);

    /// The number of callbacks.
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[ktest]
    fn softirq_and_softirqd() {
        aster_softirq::init_for_ktest();
        super::init_in_first_kthread();

        let (waiter, waker) = Waiter::new_pair();

        let softirq_line = SoftIrqLine::get(SOFTIRQ_ID);

        softirq_line.enable(move || {
            if InterruptLevel::current() != InterruptLevel::L0 {
                COUNTER_BH.fetch_add(1, Ordering::Relaxed);
            } else {
                COUNTER_DAEMON.fetch_add(1, Ordering::Relaxed);
            }
            let _ = waker.wake_up();

            if COUNTER.fetch_add(1, Ordering::Relaxed) + 1 < MAX_NUM_SOFTIRQS {
                // Raise the same softirq again.
                softirq_line.raise();
            }
        });

        softirq_line.raise();

        loop {
            waiter.wait();

            let counter_bh = COUNTER_BH.load(Ordering::Relaxed);
            let counter_daemon = COUNTER_DAEMON.load(Ordering::Relaxed);

            // There are at most `MAX_NUM_SOFTIRQS` softirqs.
            let counter = counter_bh + counter_daemon;
            assert!(counter <= MAX_NUM_SOFTIRQS);

            if counter == MAX_NUM_SOFTIRQS {
                // Since we're keeping the raise softirqs, we expect that most of the work will be
                // processed by the daemon rather than by the bottom half.
                assert!(counter_bh < counter_daemon);
                break;
            }
        }
    }
}
