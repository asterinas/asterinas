// SPDX-License-Identifier: MPL-2.0
//
// This module implements the MCS (Mellor-Crummey Scott) spinlock algorithm,
// which attribes to John Mellor-Crummey and Michael Scott for the following
// paper:
//
// John M. Mellor-Crummey and Michael L. Scott. 1991. Scalable reader-writer
// synchronization for shared-memory multiprocessors. SIGPLAN Not. 26, 7
// (July 1991), 106–113. https://doi.org/10.1145/109626.109637
//
// This implementation thankfully references the following previous work:
// [mcslock](https://crates.io/crates/mcslock), which is released under:
//
// SPDX-License-Identifier: MIT
//
// Copyright (c) 2023 Pedro de Matos Fedricci

//! MCS (Mellor-Crummey Scott) spinlock algorithm.
//!
//! This module provides only low-level primitives for the MCS spinlock
//! algorithm. And the primitives can be used to build higher-level, RAII-style
//! synchronization guards.

use core::{
    pin::Pin,
    sync::atomic::{fence, AtomicBool, AtomicPtr, Ordering},
};

/// A lock body for a MCS spinlock.
///
/// Each memory location that is shared between CPUs must have a unique
/// instance of this type.
#[derive(Debug)]
pub struct LockBody {
    tail: AtomicPtr<Node>,
}

impl LockBody {
    /// Creates a new lock body.
    ///
    /// The created lock will be in the unlocked state.
    pub const fn new() -> Self {
        Self {
            tail: AtomicPtr::new(core::ptr::null_mut()),
        }
    }
}

/// A locking node for a MCS spinlock.
///
/// Each CPU that wants to acquire the lock must instantiate a node on the
/// per-cpu storage.
///
/// The nodes are linked together as a queue. Whenever a CPU releases the lock,
/// it notifies the next CPU in the queue. So the lock serves as a fair FIFO
/// coordinator.
///
/// Each CPU only spins on its own node, so the MCS lock is cache-friendly.
///
/// # Node states
///
/// The node has two states: "acquired" and "acquiring". An "acquiring" node
/// can be used to acquire the lock. An "acquired" node lives throughout the
/// critical section and can be used to release the lock.
pub struct Node {
    /// Pointer to the next "acquiring" node in the queue.
    next: AtomicPtr<Node>,
    /// If the node state is "acquired", this field is not used.
    ///
    /// Otherwise if it is "acquiring", the predecessor will notify us by
    /// setting this field to `true`.
    ticket: AtomicBool,
    _pin: core::marker::PhantomPinned,
}

impl Node {
    /// Creates a new "acquiring" node that can be used to acquire a MCS lock.
    pub const fn new() -> Self {
        Self {
            next: AtomicPtr::new(core::ptr::null_mut()),
            ticket: AtomicBool::new(false),
            _pin: core::marker::PhantomPinned,
        }
    }

    /// Locks the MCS lock.
    ///
    /// This method spins until the lock is acquired. It can be used to
    /// synchronize critical sections. The critical section should go after
    /// this method.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the node is "acquiring". After calling this
    /// method, the node will be in the "acquired" state.
    pub unsafe fn lock(self: Pin<&Self>, lock: &LockBody) {
        let node_ptr = &*self as *const Self as *mut Self;

        let pred = lock.tail.swap(node_ptr, Ordering::AcqRel);
        // If we have a predecessor, complete the link so it will notify us.
        if !pred.is_null() {
            // SAFETY: Already verified that predecessor is not null.
            unsafe { &*pred }.next.store(node_ptr, Ordering::Release);
            while !self.ticket.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
            self.ticket.store(false, Ordering::Relaxed);
            fence(Ordering::Acquire);
        }
    }

    /// Tries locking the lock.
    ///
    /// If the lock is acquired, returns `true`. Otherwise, returns `false`.
    /// Critical sections commence after this method only if it returns `true`.
    ///
    /// Looping over `try_lock` is not recommended. The caller would be least
    /// prioritized to acquire the lock. Also it spins on a global lock, which
    /// will cause the cache line of this CPU to be invalidated frequently.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the node is "acquiring". After calling this
    /// method, the node will be in the "acquired" state if it returns `true`.
    #[expect(dead_code)]
    pub unsafe fn try_lock(self: Pin<&Self>, lock: &LockBody) -> bool {
        let node_ptr = &*self as *const Self as *mut Self;

        lock.tail
            .compare_exchange(
                core::ptr::null_mut(),
                node_ptr,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .is_ok()
    }

    /// Unlocks the lock.
    ///
    /// Critical sections should be finished before calling this method.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the node is "acquired". After calling this
    /// method, the node will be in the "acquiring" state.
    ///
    /// The caller must also ensure that the node matches the lock that it has
    /// acquired from.
    pub unsafe fn unlock(self: Pin<&Self>, lock: &LockBody) {
        let node_ptr = &*self as *const Self as *mut Self;

        let mut next = self.next.load(Ordering::Relaxed);
        if next.is_null() {
            // We don't have a known successor currently.
            if Self::try_unlock_as_tail(node_ptr, lock) {
                // And we are the tail, then dequeue and free the lock.
                return;
            }
            // But if we are not the tail, then we have a pending successor. We
            // must wait for them to finish linking with us.
            loop {
                next = self.next.load(Ordering::Relaxed);
                if !next.is_null() {
                    break;
                }
                core::hint::spin_loop();
            }
        }
        fence(Ordering::Acquire);
        // Notify the successor that we are done.
        // SAFETY: We already verified that our successor is not null.
        unsafe { &*next }.ticket.store(true, Ordering::Release);
    }

    /// FOR PFQ only.
    pub unsafe fn wake_up(self: Pin<&Self>) {
        self.ticket.store(true, Ordering::Release);
    }

    /// FOR PFQ only.
    pub unsafe fn is_blocked(self: Pin<&Self>) -> bool {
        !self.ticket.load(Ordering::Relaxed)
    }

    /// Unlocks the lock if the candidate node is the queue's tail.
    fn try_unlock_as_tail(node_ptr: *mut Self, lock: &LockBody) -> bool {
        lock.tail
            .compare_exchange(
                node_ptr,
                core::ptr::null_mut(),
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_ok()
    }
}
