// SPDX-License-Identifier: MPL-2.0

//! Queued spin lock algorithm.
//!
//! This module provides only low-level primitives for the queued spin lock
//! algorithm. And the primitives can be used to build higher-level, RAII-style
//! synchronization guards.
//!
//! This queued spin lock implementation is based on the MCS lock, but it
//! allows optimistic spinning when there's no contention observed. When
//! there's contention, it will fall back to the MCS lock for queueing.
//!
//! Comparing to the original MCS implementation, the benefit is that it does
//! not require holding some pinned allocated memory for MCS nodes in the
//! critical section. So this implementation can be heap-free/doesn't require
//! pinned stack memory. It uses static per-CPU memory for allocating temporary
//! MCS nodes. Nodes aren't needed when the lock is held.
//!
//! This implementation is inspired by the [Linux queued-spinlock]
//! (https://github.com/torvalds/linux/blob/master/kernel/locking/qspinlock.c).
//! However, due to that torn atomic memory accesses in Rust is undefined
//! behavior, the implemented algorithm is different from that in Linux, and is
//! simpler.

use core::{
    fmt::Debug,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering},
};

use crate::{cpu::CpuId, cpu_local, cpu_local_cell, task::atomic_mode::InAtomicMode};

/// A lock body for a queue spinlock.
///
/// Each memory location that is shared between CPUs must have a unique
/// instance of this type.
#[derive(Debug)]
#[repr(C)]
pub(crate) struct LockBody {
    /// Layout of the value:
    ///
    /// ```text
    /// Bits:
    /// |31            10|9          8| |7      0|
    /// +----------------+------------+ +--------+
    /// | tail CPU ID +1 | tail index | | locked |
    /// +----------------+------------+ +--------+
    /// ```
    ///
    /// Tail CPU ID and tail index together points to a MCS queue node, which
    /// is the tail of the queue.
    ///
    /// Tail CPU ID is added by 1 so that a non-zero value means someone is
    /// queued or someone has taken the lock.
    val: AtomicU32,
}

impl LockBody {
    /// Creates a new queued spinlock, which is unlocked.
    pub(crate) const fn new() -> Self {
        Self {
            val: AtomicU32::new(Self::UNLOCKED_VAL),
        }
    }

    /// Tries to lock the queued spinlock.
    ///
    /// If the lock is acquired successfully, return true.
    pub(crate) fn try_lock(&self, _guard: &dyn InAtomicMode) -> bool {
        self.val
            .compare_exchange_weak(
                Self::UNLOCKED_VAL,
                Self::LOCKED_VAL,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
    }

    /// Locks the queued spinlock.
    ///
    /// This function will spin until the lock is acquired. When the function
    /// returns, the critical section can commence.
    pub(crate) fn lock(&self, guard: &dyn InAtomicMode) {
        if !self.try_lock(guard) {
            // Slow path. There are others that are queued or the lock is
            // held by someone else.
            self.lock_enqueue(guard);
        }
    }

    /// Unlocks the queued spin-lock.
    ///
    /// This function will release the lock and allow other CPUs to acquire the
    /// lock. Call this function after the critical section is finished.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the lock is held. Unlocking a lock that is
    /// not locked is undefined behavior.
    pub(crate) unsafe fn unlock(&self) {
        let old = self.val.fetch_xor(Self::LOCKED_VAL, Ordering::Release);
        debug_assert_eq!(old & Self::LOCKED_VAL, Self::LOCKED_VAL);
    }

    /// Acquires the spinlock in a slow path.
    ///
    /// If we cannot lock optimistically by one atomic operation, we enqueue
    /// ourselves and spin on the local MCS node.
    fn lock_enqueue(&self, guard: &dyn InAtomicMode) {
        // No race because we are in atomic mode.
        let cur_cpu = CpuId::current_racy();

        let top = TOP.load();
        TOP.store(top + 1);

        macro_rules! cleanup_and_return {
            () => {
                TOP.store(top);
                return;
            };
        }

        if top >= MAX_NESTED_ACQUIRE_DEPTH {
            // If this function (`lock_queue`) is interrupted by either IRQ/
            // softirq/NMI, and the interrupt handler tries to acquire other
            // spin-locks and gets into the same situation, we may end up with
            // exceeding the maximum nested queued-spin-lock acquire depth.
            // This is extremely unlikely to happen.
            crate::early_println!("Exceeded the maximum nested queued-spin-lock acquire depth");
            // Fall back to spinning on the global lock.
            while !self.try_lock(guard) {
                core::hint::spin_loop();
            }
            cleanup_and_return!();
        }

        let node = &QUEUE_NODES.get_on_cpu(cur_cpu)[top];

        // Prevent the compiler from reordering the write to `TOP` after the
        // initialization of the node. This will likely to become a problem
        // if interrupts happen between the write to `node` and the write
        // to `TOP`.
        core::sync::atomic::compiler_fence(Ordering::Release);

        // Initialize the node.
        node.waken.store(false, Ordering::Relaxed);
        node.next.store(core::ptr::null_mut(), Ordering::Relaxed);

        // We touched a (possibly) cold cacheline in the per-CPU queue node;
        // attempt the try lock once more in the hope someone let go while we
        // weren't watching.
        if self.try_lock(guard) {
            cleanup_and_return!();
        }

        // Publish the updated tail.
        let this_node_encoded = Self::encode_tail(cur_cpu, top);
        debug_assert_ne!(this_node_encoded, Self::UNLOCKED_VAL);
        // The release ordering ensures that others read our initialized node.
        let old_val = self
            .val
            .swap(this_node_encoded | Self::LOCKED_VAL, Ordering::Release);

        // Clear the locked bit from the old value to get the old tail.
        if old_val & Self::LOCKED_VAL == 0 {
            // This means that we have swapped and set the locked bit when the
            // owner has unlocked before us and the head hasn't set the locked
            // bit. We must clear the locked bit to unblock the head.
            let read_again = self.val.fetch_xor(Self::LOCKED_VAL, Ordering::Relaxed);
            // Assert that it should still be locked before we have cleared it.
            // Because the head can't have locked it yet so it can't clear it.
            debug_assert_eq!(read_again & Self::LOCKED_VAL, Self::LOCKED_VAL);
        }
        let old_tail = old_val & !Self::LOCKED_VAL;

        // If the old tail was not null, link ourself into the queue.
        if old_tail != 0 {
            let prev_node = Self::get_tail(old_tail);

            // Link the node into the queue so that the predecessor can notify us.
            prev_node
                .next
                .store((node as *const QueueNode).cast_mut(), Ordering::Relaxed);

            // Spin on the locked bit.
            while !node.waken.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }

        // We're now at the head of the queue, wait for the owner to go away.
        let val = loop {
            let val = self.val.load(Ordering::Relaxed);
            if val & Self::LOCKED_VAL == 0 {
                break val;
            }
            core::hint::spin_loop();
        };

        // If we are the only one in the queue, claim the lock immediately.
        if val == this_node_encoded
            && self
                .val
                .compare_exchange(
                    this_node_encoded,
                    Self::LOCKED_VAL,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
        {
            cleanup_and_return!();
        }

        // Somebody else has queued after us. Wake it up and claim the lock.

        // See the above publishing swap. We have to wait the locked bit to
        // become zero again.
        while self.val.load(Ordering::Relaxed) & Self::LOCKED_VAL != 0 {
            core::hint::spin_loop();
        }

        // Actually claim the lock. This acquire pairs with the release write
        // in `unlock` to ensure that former critical section writes are
        // visible to us.
        self.val.fetch_or(Self::LOCKED_VAL, Ordering::Acquire);

        // Notify the next node. Wait for the link if not observed yet.
        let mut next: *mut QueueNode = core::ptr::null_mut();
        while next.is_null() {
            next = node.next.load(Ordering::Relaxed);
            core::hint::spin_loop();
        }
        // SAFETY: The next node is guaranteed to be valid since it is either
        // NULL (initialized) or set by the successor, which is still spinning
        // thus being alive.
        let next_node = unsafe { &*next };
        next_node.waken.store(true, Ordering::Relaxed);

        cleanup_and_return!();
    }

    const LOCKED_VAL: u32 = 1;
    const UNLOCKED_VAL: u32 = 0;

    const TAIL_TOP_OFFSET: u32 = 8;
    const TAIL_TOP_BITS: u32 = Self::CPU_ID_OFFSET - Self::TAIL_TOP_OFFSET;
    const TAIL_TOP_MASK: u32 = (1 << Self::TAIL_TOP_BITS) - 1;

    const CPU_ID_OFFSET: u32 = 10;
    const CPU_ID_BITS: u32 = 32 - Self::CPU_ID_OFFSET;
    const MAX_CPU_ID: u32 = (1 << Self::CPU_ID_BITS) - 1;

    fn encode_tail(cpu: CpuId, top: usize) -> u32 {
        debug_assert!(cpu.as_usize() < Self::MAX_CPU_ID as usize);
        ((cpu.as_usize() as u32 + 1) << Self::CPU_ID_OFFSET)
            | ((top as u32) << Self::TAIL_TOP_OFFSET)
    }

    fn get_tail(tail: u32) -> &'static QueueNode {
        let cpu = (tail >> Self::CPU_ID_OFFSET) - 1;
        let cpu_id = CpuId::try_from(cpu as usize).unwrap();

        let index = (tail >> Self::TAIL_TOP_OFFSET) & Self::TAIL_TOP_MASK;

        &QUEUE_NODES.get_on_cpu(cpu_id)[index as usize]
    }
}

struct QueueNode {
    next: AtomicPtr<QueueNode>,
    waken: AtomicBool,
}

impl QueueNode {
    const fn new() -> Self {
        Self {
            next: AtomicPtr::new(core::ptr::null_mut()),
            waken: AtomicBool::new(false),
        }
    }
}

/// The number of maximum nested queued acquire depth.
const MAX_NESTED_ACQUIRE_DEPTH: usize = 1 << LockBody::TAIL_TOP_BITS;

// Per-CPU queue node structures; it is expected that we can never have more
// than 4 nested contexts.
cpu_local! {
    static QUEUE_NODES: [QueueNode; MAX_NESTED_ACQUIRE_DEPTH] = [const { QueueNode::new() }; MAX_NESTED_ACQUIRE_DEPTH];
}

cpu_local_cell! {
    static TOP: usize = 0;
}

#[cfg(ktest)]
mod test {
    use core::sync::atomic::AtomicUsize;

    use super::*;
    use crate::{
        prelude::*,
        task::{disable_preempt, Task, TaskOptions},
    };

    #[ktest]
    fn test_mutual_exclusion() {
        static TESTED_LOCK: LockBody = LockBody::new();
        // The counter is not atomically incremented, but under lock. If the lock
        // is not mutually exclusive, the counter should be less then expected.
        static COUNTER: AtomicUsize = AtomicUsize::new(0);

        const ITERATIONS_PER_TASK: usize = 1000;
        const TASKS: usize = 4;

        static FINISHED: [AtomicBool; TASKS] = [const { AtomicBool::new(false) }; TASKS];

        fn task_fn() {
            for _ in 0..ITERATIONS_PER_TASK {
                let guard = disable_preempt();

                TESTED_LOCK.lock(&guard);

                let counter = COUNTER.load(Ordering::Relaxed);
                for _ in 0..100 {
                    assert!(!TESTED_LOCK.try_lock(&guard));
                    core::hint::spin_loop();
                }
                COUNTER.store(counter + 1, Ordering::Relaxed);

                // SAFETY: This is locked by us.
                unsafe { TESTED_LOCK.unlock() };

                drop(guard);

                Task::yield_now();
            }

            let cur_task = Task::current().unwrap();
            let tid = cur_task.local_data().downcast_ref::<usize>().unwrap();
            FINISHED[*tid].store(true, Ordering::Relaxed);
        }

        let _tasks: [Arc<Task>; TASKS] =
            core::array::from_fn(|i| TaskOptions::new(task_fn).local_data(i).spawn().unwrap());

        for finished in FINISHED.iter() {
            while !finished.load(Ordering::Relaxed) {
                Task::yield_now();
            }
        }

        let expected = TASKS * ITERATIONS_PER_TASK;
        let actual = COUNTER.load(Ordering::Relaxed);
        assert_eq!(expected, actual);
    }
}
