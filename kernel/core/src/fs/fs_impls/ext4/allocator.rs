// SPDX-License-Identifier: MPL-2.0

//! Capacity admission for parallel block-group allocation.

use core::sync::atomic::{AtomicUsize, Ordering};

use super::prelude::*;
use crate::thread::Thread;

/// Reserves free capacity without holding a lock during bitmap searches.
/// Allocation operations share the sync gate; metadata sync takes it exclusively
/// to observe complete bitmap and superblock-count updates.
#[derive(Debug)]
pub(super) struct BlockAllocator {
    capacity: Mutex<Capacity>,
    sync: RwMutex<()>,
    next_group: AtomicUsize,
}

#[derive(Debug)]
struct Capacity {
    free: u32,
    inflight: u32,
}

/// A transient capacity claim, returned automatically unless consumed.
pub(super) struct Reservation<'a> {
    allocator: &'a BlockAllocator,
    count: u32,
}

impl BlockAllocator {
    pub(super) fn new(free: u32) -> Self {
        Self {
            capacity: Mutex::new(Capacity { free, inflight: 0 }),
            sync: RwMutex::new(()),
            next_group: AtomicUsize::new(0),
        }
    }

    pub(super) fn operation(&self) -> RwMutexReadGuard<'_, ()> {
        self.sync.read()
    }

    pub(super) fn synchronize(&self) -> RwMutexWriteGuard<'_, ()> {
        self.sync.write()
    }

    pub(super) fn claim(&self, count: u32, reserved_floor: u32) -> Result<Reservation<'_>> {
        loop {
            let mut capacity = self.capacity.lock();
            let reusable = capacity.free.saturating_sub(reserved_floor);
            let granted = count.min(reusable.saturating_sub(capacity.inflight));
            if granted != 0 {
                capacity.inflight += granted;
                return Ok(Reservation {
                    allocator: self,
                    count: granted,
                });
            }
            if reusable == 0 || capacity.inflight == 0 {
                return_errno_with_message!(Errno::ENOSPC, "no free block capacity");
            }
            // A competing claim may return unused capacity after its search.
            drop(capacity);
            Thread::yield_now();
        }
    }

    pub(super) fn release(&self, count: u32) {
        let mut capacity = self.capacity.lock();
        capacity.free = capacity.free.checked_add(count).unwrap();
    }

    /// Reconciles capacity with group totals while holding the exclusive sync gate.
    pub(super) fn reconcile(&self, free: u32) {
        let mut capacity = self.capacity.lock();
        assert_eq!(capacity.inflight, 0);
        capacity.free = free;
    }

    /// Spreads independent trees' fallback goals; no blocks are preallocated.
    pub(super) fn home_group(&self, nr_groups: usize) -> usize {
        self.next_group.fetch_add(1, Ordering::Relaxed) % nr_groups
    }
}

impl Reservation<'_> {
    pub(super) fn count(&self) -> u32 {
        self.count
    }

    pub(super) fn consume(mut self, count: u32) {
        let mut capacity = self.allocator.capacity.lock();
        assert!(count <= self.count);
        capacity.free -= count;
        capacity.inflight -= self.count;
        self.count = 0;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.count != 0 {
            self.allocator.capacity.lock().inflight -= self.count;
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn capacity_claims_preserve_reserves_and_return_unused_space() {
        let allocator = BlockAllocator::new(10);
        let claim = allocator.claim(8, 2).unwrap();
        assert_eq!(claim.count(), 8);
        claim.consume(3);
        let claim = allocator.claim(8, 2).unwrap();
        assert_eq!(claim.count(), 5);
        claim.consume(5);
        assert!(allocator.claim(1, 2).is_err());
        allocator.release(8);
        drop(allocator.claim(10, 0).unwrap());
        let claim = allocator.claim(10, 0).unwrap();
        assert_eq!(claim.count(), 10);
        claim.consume(10);
        assert!(allocator.claim(1, 0).is_err());
        let _synchronization = allocator.synchronize();
        allocator.reconcile(4);
        let claim = allocator.claim(4, 0).unwrap();
        assert_eq!(claim.count(), 4);
        claim.consume(4);
    }
}
