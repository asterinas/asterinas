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
    cell::UnsafeCell,
    sync::atomic::{fence, AtomicBool, AtomicPtr, Ordering},
};

use crate::{cpu_local, cpu_local_cell};

/// A lock body for a MCS spinlock.
///
/// Each memory location that is shared between CPUs must have a unique
/// instance of this type.
pub(crate) struct LockBody {
    tail: AtomicPtr<Node>,
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
pub(crate) struct NodeRef {
    node_ptr: *mut Node,
}

impl NodeRef {
    /// Allocates a new node and returns a reference to it.
    ///
    /// # Safety
    ///
    /// The caller must ensure that preemption is disabled throughout the
    /// lifetime of the allocated node.
    pub(crate) unsafe fn alloc() -> Self {
        let top = NODE_ALLOC_TOP.load();
        if top >= MAX_NESTED_LOCKS {
            panic!("MCS lock nested too deep");
        }
        NODE_ALLOC_TOP.add_assign(1);
        Self {
            // SAFETY: preemption is disabled. And the pointer is valid.
            node_ptr: unsafe { (NODES.as_ptr() as *const _ as *mut Node).add(top) },
        }
    }
}

impl Drop for NodeRef {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            // We can check if the lock is dropped in the reverse order of acquisition.

            // SAFETY: preemption is disabled.
            let nodes_base = unsafe { NODES.as_ptr() } as *mut Node;
            // SAFETY: The node pointer is derived from the nodes array pointer
            // with a valid offset.
            let node_offset = unsafe { self.node_ptr.offset_from(nodes_base) };

            let cur = NODE_ALLOC_TOP.load() as isize - 1;

            debug_assert!(node_offset == cur, "MCS lock not dropped in order");
        }
        NODE_ALLOC_TOP.sub_assign(1);
    }
}

impl NodeRef {
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
    pub(crate) unsafe fn lock(&mut self, lock: &LockBody) {
        // SAFETY: The pointer is valid and we have exclusive access to it.
        let node = unsafe { &*self.node_ptr };

        let pred = lock.tail.swap(self.node_ptr, Ordering::AcqRel);
        // If we have a predecessor, complete the link so it will notify us.
        if !pred.is_null() {
            // SAFETY: Already verified that predecessor is not null.
            unsafe { &*pred }
                .next
                .store(self.node_ptr, Ordering::Release);
            while !node.ticket.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
            node.ticket.store(false, Ordering::Relaxed);
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
    pub(crate) unsafe fn try_lock(&mut self, lock: &LockBody) -> bool {
        lock.tail
            .compare_exchange(
                core::ptr::null_mut(),
                self.node_ptr,
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
    pub(crate) unsafe fn unlock(&mut self, lock: &LockBody) {
        // SAFETY: The pointer is valid and we have exclusive access to it.
        let node = unsafe { &*self.node_ptr };

        let mut next = node.next.load(Ordering::Relaxed);
        if next.is_null() {
            // We don't have a known successor currently.
            if self.try_unlock_as_tail(lock) {
                // And we are the tail, then dequeue and free the lock.
                return;
            }
            // But if we are not the tail, then we have a pending successor. We
            // must wait for them to finish linking with us.
            loop {
                next = node.next.load(Ordering::Relaxed);
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

    /// Unlocks the lock if the candidate node is the queue's tail.
    fn try_unlock_as_tail(&mut self, lock: &LockBody) -> bool {
        lock.tail
            .compare_exchange(
                self.node_ptr,
                core::ptr::null_mut(),
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_ok()
    }
}

/// The actual node data structure.
///
/// We allocate the nodes in a CPU-local fixed-size array. Although the better
/// way is to allocate them on the stack just like what everyone else do, it is
/// hard to do so because the node should be pinned ([`core::pin::pin!`]),
/// which forces us to use the `with`-style API rather than the guard-style API.
///
/// However, a bright side is that when we allocate the nodes by ourselves, we
/// can detect the drop order of the nodes, which would be helpful preventing
/// subtle deadlocks introduced by the RAII-style API.
struct Node {
    /// Pointer to the next "acquiring" node in the queue.
    next: AtomicPtr<Node>,
    /// If the node state is "acquired", this field is not used.
    ///
    /// Otherwise if it is "acquiring", the predecessor will notify us by
    /// setting this field to `true`.
    ticket: AtomicBool,
}

// TODO: just like stack size, this should be configurable.
const MAX_NESTED_LOCKS: usize = 64;

cpu_local! {
    static NODES: UnsafeCell<[Node; MAX_NESTED_LOCKS]> = UnsafeCell::new([const { Node::new() }; MAX_NESTED_LOCKS]);
}

cpu_local_cell! {
    static NODE_ALLOC_TOP: usize = 0;
}

/// Resets the node allocation info for the application processor (AP).
///
/// It exists because that the BSP may acquire locks early before the CPU-local
/// storage for APs is initialized. So the AP may boot with the CPU local
/// storage dirty.
///
/// # Safety
///
/// It should only be called on the AP and only once when initializing the AP's
/// CPU-local storage.
pub(crate) unsafe fn reset_node_alloc_info_for_ap() {
    NODE_ALLOC_TOP.store(0);
}

impl Node {
    /// Creates a new "acquiring" node that can be used to acquire a MCS lock.
    const fn new() -> Self {
        Self {
            next: AtomicPtr::new(core::ptr::null_mut()),
            ticket: AtomicBool::new(false),
        }
    }
}
