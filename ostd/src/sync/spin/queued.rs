// SPDX-License-Identifier: MPL-2.0
//
// This module provides a Rust-implementation of the [Linux queued-spinlock]
// (https://github.com/torvalds/linux/blob/master/kernel/locking/qspinlock.c),
// which is released under:
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
// Original copyright remarks:
//
// (C) Copyright 2013-2015 Hewlett-Packard Development Company, L.P.
// (C) Copyright 2013-2014,2018 Red Hat, Inc.
// (C) Copyright 2015 Intel Corp.
// (C) Copyright 2015 Hewlett-Packard Enterprise Development LP
//
// Original Authors: Waiman Long <longman@redhat.com>
//                   Peter Zijlstra <peterz@infradead.org>
//
// Authors of this Rust adaptation:
//                   Junyang Zhang <junyang@stu.pku.edu.cn>

//! Queued spin lock algorithm.
//!
//! This module provides only low-level primitives for the queued spin lock
//! algorithm. And the primitives can be used to build higher-level, RAII-style
//! synchronization guards.
//!
//! This queued spin lock implementation is based on the MCS lock, but it
//! allows optimistic spinning when there's no contention observed. When
//! there's contention, it will fall back to the MCS lock for queueing.

use core::{
    cell::UnsafeCell,
    fmt::Debug,
    intrinsics,
    mem::{offset_of, ManuallyDrop},
    sync::atomic::{AtomicBool, AtomicPtr, Ordering},
};

use crate::{
    cpu::{CpuId, PinCurrentCpu},
    cpu_local, cpu_local_cell,
    task::atomic_mode::InAtomicMode,
};

/// A lock body for a queue spinlock.
///
/// Each memory location that is shared between CPUs must have a unique
/// instance of this type.
#[repr(C)]
pub(crate) union LockBody {
    /// Layout of the value:
    ///
    /// ```text
    /// Bits:
    /// |31            18|17        16| |        8| |       0|
    /// +----------------+------------+ +---------+ +--------+
    /// | tail CPU ID +1 | tail index | | pending | | locked |
    /// +----------------+------------| +---------+ +--------+
    /// Bytes (little-endian):
    /// |     3    |       2          | |    1    | |    0   |
    /// Bytes (big-endian):
    /// |     0    |       1          | |    2    | |    3   |
    /// ```
    val: ManuallyDrop<UnsafeCell<u32>>,
    /// We would access the fields separately to optimize atomic operations.
    inner_repr1: ManuallyDrop<InnerRepr1>,
    /// We would access the locked and pending bits together to optimize
    /// atomic operations.
    inner_repr2: ManuallyDrop<InnerRepr2>,
}

// SAFETY: The structure will be used for synchronization by design.
unsafe impl Sync for LockBody {}
// SAFETY: When sending a lock body there will be no shared references to it.
// So there's no race that could happen.
unsafe impl Send for LockBody {}

#[cfg(target_endian = "little")]
#[repr(C)]
struct InnerRepr1 {
    locked: UnsafeCell<u8>,
    pending: UnsafeCell<u8>,
    tail: UnsafeCell<u16>,
}

#[cfg(target_endian = "little")]
#[repr(C)]
struct InnerRepr2 {
    locked_pending: UnsafeCell<u16>,
    _tail: UnsafeCell<u16>,
}

#[cfg(target_endian = "big")]
#[repr(C)]
struct InnerRepr1 {
    tail: UnsafeCell<u16>,
    pending: UnsafeCell<u8>,
    locked: UnsafeCell<u8>,
}

#[cfg(target_endian = "big")]
#[repr(C)]
struct InnerRepr2 {
    _tail: UnsafeCell<u16>,
    locked_pending: UnsafeCell<u16>,
}

impl LockBody {
    const LOCKED_OFFSET: usize = offset_of!(InnerRepr1, locked) * 8;
    const LOCKED_VAL: u32 = 0x1 << Self::LOCKED_OFFSET;
    const LOCKED_MASK: u32 = 0xff << Self::LOCKED_OFFSET;

    const PENDING_OFFSET: usize = offset_of!(InnerRepr1, pending) * 8;
    const PENDING_VAL: u32 = 0x1 << Self::PENDING_OFFSET;
    const PENDING_MASK: u32 = 0xff << Self::PENDING_OFFSET;

    const TAIL_OFFSET: usize = offset_of!(InnerRepr1, tail) * 8;
    const TAIL_MASK: u32 = 0xffff << Self::TAIL_OFFSET;
    const TAIL_TOP_BITS: usize = 2;
    const TAIL_TOP_MASK_IN_TAIL: u16 = (0x1 << Self::TAIL_TOP_BITS) - 1;
    const MAX_CPU_COUNT: usize = (u16::MAX as usize >> Self::TAIL_TOP_BITS) - 1;

    fn encode_tail(cpu: CpuId, top: usize) -> u16 {
        debug_assert!(cpu.as_usize() < Self::MAX_CPU_COUNT);
        ((cpu.as_usize() as u16 + 1) << Self::TAIL_TOP_BITS)
            | (top as u16 & Self::TAIL_TOP_MASK_IN_TAIL)
    }

    fn get_tail(tail: u16) -> &'static QueueNode {
        let cpu = (tail >> Self::TAIL_TOP_BITS) - 1;
        let cpu_id = CpuId::try_from(cpu as usize).unwrap();

        let index = tail & Self::TAIL_TOP_MASK_IN_TAIL;

        &QUEUE_NODES.get_on_cpu(cpu_id)[index as usize]
    }
}

impl InnerRepr2 {
    #[cfg(target_endian = "little")]
    const FIRST_OFFSET: usize = 0;
    #[cfg(target_endian = "big")]
    const FIRST_OFFSET: usize = 16;

    const LOCKED_OFFSET: usize = offset_of!(InnerRepr1, locked) * 8 - Self::FIRST_OFFSET;
    const LOCKED_VAL: u16 = 0x1 << Self::LOCKED_OFFSET;
    #[expect(dead_code)]
    const LOCKED_MASK: u16 = 0xff << Self::LOCKED_OFFSET;

    const PENDING_OFFSET: usize = offset_of!(InnerRepr1, pending) * 8 - Self::FIRST_OFFSET;
    #[expect(dead_code)]
    const PENDING_VAL: u16 = 0x1 << Self::PENDING_OFFSET;
    #[expect(dead_code)]
    const PENDING_MASK: u16 = 0xff << Self::PENDING_OFFSET;
}

impl Debug for LockBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let val = unsafe { intrinsics::atomic_load_relaxed(self.val_ptr()) };
        f.debug_struct("LockBody")
            .field("locked", &(val & LockBody::LOCKED_MASK != 0))
            .field("pending", &(val & LockBody::PENDING_MASK != 0))
            .field("tail", &(val >> LockBody::TAIL_OFFSET))
            .finish()
    }
}

impl LockBody {
    /// Create a new queued spinlock, which is unlocked.
    pub(crate) const fn new() -> Self {
        Self {
            val: ManuallyDrop::new(UnsafeCell::new(0)),
        }
    }

    /// Try to lock the queued spinlock.
    ///
    /// If the lock is acquired successfully, return an acquired guard.
    /// Otherwise, return a guard that haven't acquired the lock.
    ///
    /// If it returns an acquired guard the critical section can commence.
    pub(crate) fn try_lock(&self, guard: &dyn InAtomicMode) -> bool {
        let _ = guard;
        self.try_lock_impl().is_ok()
    }

    /// Unlock the queued spin-lock.
    ///
    /// This function will release the lock and allow other CPUs to acquire the
    /// lock. Call this function after the critical section is finished.
    ///
    /// # Safety
    ///
    /// The caller must ensure the following properties:
    ///  - The lock is pinned after any one acquired it and before every one
    ///    unlocks it (i.e. no lock-stealing).
    ///  - The lock was acquired by the current CPU for once and not unlocked.
    pub(crate) unsafe fn unlock(&self) {
        // 0,0,1 -> 0,0,0
        unsafe {
            intrinsics::atomic_store_release(self.locked_ptr(), 0);
        }
    }

    /// Lock the queued spinlock.
    ///
    /// This function will spin until the lock is acquired. When the function
    /// returns, the critical section can commence.
    pub(crate) fn lock(&self, guard: &dyn InAtomicMode) {
        // (queue tail, pending bit, lock value)
        //
        //              fast     :    slow                                  :    unlock
        //                       :                                          :
        // uncontended  (0,0,0) -:--> (0,0,1) ------------------------------:--> (*,*,0)
        //                       :       | ^--------.------.             /  :
        //                       :       v           \      \            |  :
        // pending               :    (0,1,1) +--> (0,1,0)   \           |  :
        //                       :       | ^--'              |           |  :
        //                       :       v                   |           |  :
        // uncontended           :    (n,x,y) +--> (n,0,0) --'           |  :
        //   queue               :       | ^--'                          |  :
        //                       :       v                               |  :
        // contended             :    (*,x,y) +--> (*,0,0) ---> (*,0,1) -'  :
        //   queue               :         ^--'                             :

        match self.try_lock_impl() {
            Ok(()) => {}
            Err(val) => {
                // Slow path. There are pending spinners or queued spinners.
                self.lock_slow(val, guard);
            }
        }
    }

    /// Acquire the spinlock in a slow path.
    fn lock_slow(&self, mut lock_val: u32, guard: &dyn InAtomicMode) {
        // Wait for in-progress pending->locked hand-overs with a bounded
        // number of spins so that we guarantee forward progress.
        // 0,1,0 -> 0,0,1
        if lock_val == Self::PENDING_VAL {
            /// The pending bit spinning loop count.
            const QUEUE_PENDING_LOOPS: usize = 1;

            for _ in 0..QUEUE_PENDING_LOOPS {
                core::hint::spin_loop();
                lock_val = unsafe { intrinsics::atomic_load_relaxed(self.val_ptr()) };
                if lock_val != Self::PENDING_VAL {
                    break;
                }
            }
        }

        // If we observe any contention, we enqueue ourselves.
        if lock_val & !Self::LOCKED_MASK != 0 {
            self.lock_enqueue(guard);
            return;
        }

        // 0,0,* -> 0,1,* -> 0,0,1 pending, trylock
        lock_val = unsafe { intrinsics::atomic_or_acquire(self.val_ptr(), Self::PENDING_VAL) };

        // If we observe contention, there is a concurrent locker.
        if lock_val & !Self::LOCKED_MASK != 0 {
            // Undo the pending bit if we set it.
            if (lock_val & Self::PENDING_MASK) == 0 {
                unsafe {
                    self.pending_ptr().write_volatile(0);
                };
            }
            self.lock_enqueue(guard);
            return;
        }

        // We're pending, wait for the owner to go away.
        // 0,1,1 -> *,1,0
        if lock_val & Self::LOCKED_MASK != 0 {
            while unsafe { intrinsics::atomic_load_acquire(self.locked_ptr()) } != 0 {
                core::hint::spin_loop();
            }
        }

        // Take the ownership and clear the pending bit.
        // 0,1,0 -> 0,0,1
        unsafe {
            self.locked_pending_ptr()
                .write_volatile(InnerRepr2::LOCKED_VAL)
        };

        // We're done.
    }

    /// Try to lock the spinlock in an optimistic path.
    ///
    /// If the lock is acquired successfully, return `Ok(())`. Otherwise, return
    /// `Err(prev_val)`.
    fn try_lock_impl(&self) -> Result<(), u32> {
        let (val, ret) = unsafe {
            intrinsics::atomic_cxchg_acquire_relaxed(self.val_ptr(), 0, LockBody::LOCKED_VAL)
        };
        if ret {
            Ok(())
        } else {
            Err(val)
        }
    }

    /// Acquire the spinlock in a very-slow path.
    ///
    /// If we cannot lock optimistically (likely due to contention), we enqueue
    /// ourselves and spin on local MCS nodes.
    fn lock_enqueue(&self, guard: &dyn InAtomicMode) {
        let cur_cpu = guard.current_cpu();

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
            while self.try_lock_impl().is_err() {
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
        node.locked.store(false, Ordering::Relaxed);
        node.next.store(core::ptr::null_mut(), Ordering::Relaxed);

        // We touched a (possibly) cold cacheline in the per-CPU queue node;
        // attempt the try lock once more in the hope someone let go while we
        // weren't watching.
        if self.try_lock_impl().is_ok() {
            cleanup_and_return!();
        }

        // Ensure that the initialization of the node is complete before we
        // publish the updated tail via swap and potentially link the node
        // into the wait queue.
        core::sync::atomic::fence(Ordering::Release);

        // Publish the updated tail.
        // p,*,* -> n,*,*
        let this_node_encoded = Self::encode_tail(cur_cpu, top);
        let old_tail =
            unsafe { intrinsics::atomic_xchg_relaxed(self.tail_ptr(), this_node_encoded) };
        let mut next: *mut QueueNode = core::ptr::null_mut();

        // If the old tail was not null, link the node into the queue.
        if old_tail != 0 {
            let prev_node = Self::get_tail(old_tail);

            // Link the node into the queue so that the predecessor can notify us.
            prev_node
                .next
                .store((node as *const QueueNode).cast_mut(), Ordering::Relaxed);

            // Spin on the locked bit.
            while !node.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }

            // While waiting for the MCS lock, the next pointer may have been
            // updated. If so, we optimitically load it for writing now as we
            // may soon need it.
            next = node.next.load(Ordering::Relaxed);
            if !next.is_null() {
                unsafe {
                    intrinsics::prefetch_write_data(next.cast_const(), 0);
                }
            }
        }

        // We're now at the head of the queue, wait for the owner & pending to
        // go away.
        // *,x,y -> *,0,0

        let mut val;

        loop {
            val = unsafe { intrinsics::atomic_load_acquire(self.val_ptr()) };
            if val & (Self::LOCKED_MASK | Self::PENDING_MASK) == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // Claim the lock.
        // n,0,0 -> 0,0,1 : lock, uncontented
        // *,*,0 -> 0,0,1 : lock, contended

        if val & Self::TAIL_MASK == (this_node_encoded as u32) << Self::TAIL_OFFSET {
            // If the queue head is the only one in the queue, and nobody is
            // pending, clear the tail.
            let (_, unlocked_as_tail) = unsafe {
                intrinsics::atomic_cxchg_relaxed_relaxed(self.val_ptr(), val, Self::LOCKED_VAL)
            };
            if unlocked_as_tail {
                // Uncontended.
                cleanup_and_return!();
            }
        }

        // Contended.

        // Either somebody is queued behind us, or somebody is sets the
        // pending bit. If that one set a pending bit then it will be
        // queued behind us. So we will see a `next`.
        unsafe {
            self.locked_ptr().write_volatile(1);
        }

        // Wait for next if not observed yet.
        while next.is_null() {
            next = node.next.load(Ordering::Relaxed);
            core::hint::spin_loop();
        }

        // Notify the next node.
        // SAFETY: The next node is guaranteed to be valid.
        let next_node = unsafe { &*next };
        next_node.locked.store(true, Ordering::Release);

        cleanup_and_return!();
    }

    fn val_ptr(&self) -> *mut u32 {
        // SAFETY: All the fields in this union are pairwise transmutable.
        unsafe { self.val.get() }
    }

    fn locked_ptr(&self) -> *mut u8 {
        // SAFETY: All the fields in this union are pairwise transmutable.
        unsafe { self.inner_repr1.locked.get() }
    }

    fn pending_ptr(&self) -> *mut u8 {
        // SAFETY: All the fields in this union are pairwise transmutable.
        unsafe { self.inner_repr1.pending.get() }
    }

    fn tail_ptr(&self) -> *mut u16 {
        // SAFETY: All the fields in this union are pairwise transmutable.
        unsafe { self.inner_repr1.tail.get() }
    }

    fn locked_pending_ptr(&self) -> *mut u16 {
        // SAFETY: All the fields in this union are pairwise transmutable.
        unsafe { self.inner_repr2.locked_pending.get() }
    }
}

struct QueueNode {
    next: AtomicPtr<QueueNode>,
    locked: AtomicBool,
}

impl QueueNode {
    const fn new() -> Self {
        Self {
            next: AtomicPtr::new(core::ptr::null_mut()),
            locked: AtomicBool::new(false),
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
