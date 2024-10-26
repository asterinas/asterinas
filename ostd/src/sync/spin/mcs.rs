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
// [mcslock](https://crates.io/crates/mcslock).

//! MCS (Mellor-Crummey Scott) spinlock algorithm.
//!
//! This module provides only low-level primitives for the MCS spinlock
//! algorithm. And the primitives can be used to build higher-level, RAII-style
//! synchronization guards.

use core::{
    marker::PhantomPinned,
    pin::Pin,
    sync::atomic::{fence, AtomicBool, AtomicPtr, Ordering},
};

/// A lock body for a MCS spinlock.
///
/// Each memory location that is shared between CPUs must have a unique
/// instance of this type.
pub(crate) struct LockBody {
    tail: AtomicPtr<UnsafeNode>,
}

impl LockBody {
    /// Creates a new lock body.
    ///
    /// The created lock will be in the unlocked state.
    pub(crate) const fn new() -> Self {
        Self {
            tail: AtomicPtr::new(core::ptr::null_mut()),
        }
    }
}

/// A locking node for a MCS spinlock.
///
/// Each CPU that wants to acquire the lock must instantiate a node on the
/// stack.
///
/// The nodes are linked together as a queue. Whenever a CPU releases the lock,
/// it notifies the next CPU in the queue. So the lock serves as a fair FIFO
/// coordinator.
///
/// Each CPU only spins on its own node, so the MCS lock is cache-friendly.
///
/// Directly using this type is not safe. See [`Node`] for a safe wrapper that
/// ensures that the node state is correct at compile time.
pub(crate) struct UnsafeNode {
    /// Pointer to the next "acquiring" node in the queue.
    next: AtomicPtr<UnsafeNode>,
    /// If the node state is "acquired", this field is not used.
    ///
    /// Otherwise if it is "acquiring", the predecessor will notify this us by
    /// setting this field to `true`.
    ticket: AtomicBool,
    // The node will be accessed by pointers so it better not to be moved.
    _pin: PhantomPinned,
}

impl UnsafeNode {
    /// Creates a new unsafe node in the "ready" state.
    pub(crate) const fn new() -> Self {
        Self {
            next: AtomicPtr::new(core::ptr::null_mut()),
            ticket: AtomicBool::new(false),
            _pin: PhantomPinned,
        }
    }
}

/// Node with compile-time state checking.
///
/// See [`UnsafeNode`] for the internal representation and the usage of the node.
///
/// The node has two states: "acquired" and "ready". A "ready" node can be
/// used to acquire the lock. An "acquired" node lives throughout the critical
/// section and can be used to release the lock. After releasing the lock, the
/// node becomes "ready".
///
/// Violating the node state will lead to undefined behavior. This checker
/// ensures that the node state is correct at compile time.
///
/// If the node is "ready", the `READY` parameter should be `true`. Otherwise,
/// it should be `false`.
pub(crate) struct Node<'a, 'b, const READY: bool> {
    lock: &'a LockBody,
    node: Pin<&'b mut UnsafeNode>,
}

// "ready" node.
impl<'a, 'b> Node<'a, 'b, true> {
    /// Creates a new "ready" node that can be used to acquire a MCS lock.
    pub(crate) fn new(lock: &'a LockBody, unsafe_node: Pin<&'b mut UnsafeNode>) -> Self {
        Self {
            lock,
            node: unsafe_node,
        }
    }

    /// Locks the MCS lock.
    ///
    /// This method spins until the lock is acquired. It can be used to
    /// synchronize critical sections. The critical section should go after
    /// this method.
    pub(crate) fn lock(self) -> Node<'a, 'b, false> {
        let node_ptr = (&*self.node as *const UnsafeNode).cast_mut();

        let pred = self.lock.tail.swap(node_ptr, Ordering::AcqRel);
        // If we have a predecessor, complete the link so it will notify us.
        if !pred.is_null() {
            // SAFETY: Already verified that predecessor is not null.
            unsafe { &*pred }.next.store(node_ptr, Ordering::Release);
            while !self.node.ticket.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
            // Reset the ticket for the next user.
            self.node.ticket.store(false, Ordering::Relaxed);
            fence(Ordering::Acquire);
        }

        // The node is now "acquired".
        Node::<false> {
            lock: self.lock,
            node: self.node,
        }
    }

    /// Tries locking the lock.
    ///
    /// If the lock is acquired, returns a `Ok(Node<false>)`, which represents
    /// a "acquired" node. Otherwise, returns a `Err(Node<true>)`, which
    /// represents a "ready" node.
    ///
    /// Looping over `try_lock` is not recommended. The caller would be least
    /// prioritized to acquire the lock. Also it spins on a global lock, which
    /// will cause the cache line of this CPU to be invalidated frequently.
    pub(crate) fn try_lock(self) -> Result<Node<'a, 'b, false>, Node<'a, 'b, true>> {
        let node_ptr = (&*self.node as *const UnsafeNode).cast_mut();

        let acquired = self
            .lock
            .tail
            .compare_exchange(
                core::ptr::null_mut(),
                node_ptr,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .is_ok();

        if acquired {
            // The node is now "acquired".
            Ok(Node::<false> {
                lock: self.lock,
                node: self.node,
            })
        } else {
            // The node is still "ready".
            Err(self)
        }
    }
}

// "acquired" node.
impl<'a, 'b> Node<'a, 'b, false> {
    /// Unlocks the lock.
    ///
    /// Critical sections should be finished before calling this method.
    pub(crate) fn unlock(self) -> Node<'a, 'b, true> {
        let mut next = self.node.next.load(Ordering::Relaxed);
        if next.is_null() {
            // We don't have a known successor currently.
            if self.try_unlock_as_tail() {
                // And we are the tail, then dequeue and free the lock.
                return Node::<true> {
                    lock: self.lock,
                    node: self.node,
                };
            }
            // But if we are not the tail, then we have a pending successor. We
            // must wait for them to finish linking with us.
            loop {
                next = self.node.next.load(Ordering::Relaxed);
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

        // The node is now "ready".
        Node::<true> {
            lock: self.lock,
            node: self.node,
        }
    }

    /// Unlocks the lock if the candidate node is the queue's tail.
    fn try_unlock_as_tail(&self) -> bool {
        let node_ptr = (&*self.node as *const UnsafeNode).cast_mut();

        self.lock
            .tail
            .compare_exchange(
                node_ptr,
                core::ptr::null_mut(),
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_ok()
    }
}
