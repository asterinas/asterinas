// SPDX-License-Identifier: MPL-2.0

use id_alloc::IdAlloc;
use ostd::sync::{SpinLock, WaitQueue};

/// A synchronized ID allocator that manages a fixed-size pool of IDs.
///
/// We say this ID allocator "synchronized" because
/// its `alloc` method may be used by multiple threads to allocate IDs concurrently
/// and it would block if no free IDs are available at the moment.
/// Once an ID is no longer in use, the `dealloc` method may be called
/// to return the ID to the ID allocator.
///
/// This ID allocator is designed for use by device drivers.
/// The `alloc` method should only be called in the task context,
/// whereas the `dealloc` method should only be used in the IRQ handler of a device driver.
/// Failing to conform with the above requirement may result in deadlock.
pub struct SyncIdAlloc {
    wait_queue: WaitQueue,
    id_allocator: SpinLock<IdAlloc>,
}

impl SyncIdAlloc {
    /// Creates an allocator that may return IDs in the range of `0..capacity`.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            wait_queue: WaitQueue::new(),
            id_allocator: SpinLock::new(IdAlloc::with_capacity(capacity)),
        }
    }

    /// Allocates a new ID.
    ///
    /// This method must only be called in the task context.
    /// It will block until an ID is free to allocate.
    pub fn alloc(&self) -> usize {
        self.wait_queue
            .wait_until(|| self.id_allocator.disable_irq().lock().alloc())
    }

    /// Deallocates an ID.
    ///
    /// This method assumes that the caller is in the context of an IRQ handler.
    ///
    /// # Panics
    ///
    /// This method would panic if `id` is greater than or equal to the capacity of this ID allocator.
    pub fn dealloc(&self, id: usize) {
        self.id_allocator.disable_irq().lock().free(id);
        self.wait_queue.wake_all();
    }
}

impl core::fmt::Debug for SyncIdAlloc {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("SyncIdAlloc")
            .field("id_allocator", &self.id_allocator.lock())
            .finish_non_exhaustive()
    }
}
