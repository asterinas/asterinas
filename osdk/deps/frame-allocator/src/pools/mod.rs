// SPDX-License-Identifier: MPL-2.0

mod balancing;

use core::{
    alloc::Layout,
    cell::RefCell,
    ops::DerefMut,
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd::{
    cpu_local,
    irq::DisabledLocalIrqGuard,
    mm::Paddr,
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard},
};

use crate::chunk::{BuddyOrder, greater_order_of, lesser_order_of, size_of_order, split_to_chunks};

use super::set::BuddySet;

/// The global free buddies.
static GLOBAL_POOL: SpinLock<BuddySet<MAX_BUDDY_ORDER>, LocalIrqDisabled> =
    SpinLock::new(BuddySet::new_empty());
/// A snapshot of the total size of the global free buddies, not precise.
static GLOBAL_POOL_SIZE: AtomicUsize = AtomicUsize::new(0);

// CPU-local free buddies.
cpu_local! {
    static LOCAL_POOL: RefCell<BuddySet<MAX_LOCAL_BUDDY_ORDER>> = RefCell::new(BuddySet::new_empty());
}

/// Maximum supported order of the buddy system.
///
/// i.e., it is the number of classes of free blocks. It determines the
/// maximum size of each allocation.
///
/// A maximum buddy order of 32 supports up to 4KiB*2^31 = 8 TiB of chunks.
const MAX_BUDDY_ORDER: BuddyOrder = 32;

/// Maximum supported order of the buddy system for CPU-local buddy system.
///
/// Since large blocks are rarely allocated, caching such blocks will lead
/// to much fragmentation.
///
/// Lock guards are also allocated on stack. We can limit the stack usage
/// for common paths in this way.
///
/// A maximum local buddy order of 18 supports up to 4KiB*2^17 = 512 MiB of
/// chunks.
const MAX_LOCAL_BUDDY_ORDER: BuddyOrder = 18;

pub(super) fn alloc(guard: &DisabledLocalIrqGuard, layout: Layout) -> Option<Paddr> {
    let local_pool_cell = LOCAL_POOL.get_with(guard);
    let mut local_pool = local_pool_cell.borrow_mut();
    let mut global_pool = OnDemandGlobalLock::new();

    alloc_from(&mut local_pool, &mut global_pool, layout)
}

fn alloc_from(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global_pool: &mut OnDemandGlobalLock,
    layout: Layout,
) -> Option<Paddr> {
    let size_order = greater_order_of(layout.size());
    let align_order = greater_order_of(layout.align());
    let order = size_order.max(align_order);

    let mut chunk_addr = None;

    if order < MAX_LOCAL_BUDDY_ORDER {
        chunk_addr = local_pool.alloc_chunk(order);
    }

    // Fall back to the global free lists if the local free lists are empty.
    if chunk_addr.is_none() {
        chunk_addr = global_pool.get().alloc_chunk(order);
    }
    // TODO: On memory pressure the global pool may be not enough. We may need
    // to merge all buddy chunks from the local pools to the global pool and
    // try again.

    // If the alignment order is larger than the size order, we need to split
    // the chunk and return the rest part back to the free lists.
    let allocated_size = size_of_order(order);
    if allocated_size > layout.size()
        && let Some(chunk_addr) = chunk_addr
    {
        do_dealloc(
            local_pool,
            global_pool,
            [(chunk_addr + layout.size(), allocated_size - layout.size())].into_iter(),
        );
    }

    balancing::balance(local_pool, global_pool);

    chunk_addr
}

pub(super) fn dealloc(
    guard: &DisabledLocalIrqGuard,
    segments: impl Iterator<Item = (Paddr, usize)>,
) {
    let local_pool_cell = LOCAL_POOL.get_with(guard);
    let mut local_pool = local_pool_cell.borrow_mut();
    let mut global_pool = OnDemandGlobalLock::new();

    do_dealloc(&mut local_pool, &mut global_pool, segments);

    balancing::balance(local_pool.deref_mut(), &mut global_pool);
}

pub(super) fn add_free_memory(_guard: &DisabledLocalIrqGuard, addr: Paddr, size: usize) {
    let mut global_pool = OnDemandGlobalLock::new();

    split_to_chunks(addr, size).for_each(|(addr, order)| {
        global_pool.get().insert_chunk(addr, order);
    });
}

fn do_dealloc(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global_pool: &mut OnDemandGlobalLock,
    segments: impl Iterator<Item = (Paddr, usize)>,
) {
    segments.for_each(|(addr, size)| {
        split_to_chunks(addr, size).for_each(|(addr, order)| {
            if order >= MAX_LOCAL_BUDDY_ORDER {
                global_pool.get().insert_chunk(addr, order);
            } else {
                local_pool.insert_chunk(addr, order);
            }
        });
    });
}

type GlobalLockGuard = SpinLockGuard<'static, BuddySet<MAX_BUDDY_ORDER>, LocalIrqDisabled>;

/// An on-demand guard that locks the global pool when needed.
///
/// It helps to avoid unnecessarily locking the global pool, and also avoids
/// repeatedly locking the global pool when it is needed multiple times.
struct OnDemandGlobalLock {
    guard: Option<GlobalLockGuard>,
}

impl OnDemandGlobalLock {
    fn new() -> Self {
        Self { guard: None }
    }

    fn get(&mut self) -> &mut GlobalLockGuard {
        self.guard.get_or_insert_with(|| GLOBAL_POOL.lock())
    }

    /// Returns the size of the global pool.
    ///
    /// If the global pool is locked, returns the actual size of the global pool.
    /// Otherwise, returns the last snapshot of the global pool size by loading
    /// [`GLOBAL_POOL_SIZE`].
    fn get_global_size(&self) -> usize {
        if let Some(guard) = self.guard.as_ref() {
            guard.total_size()
        } else {
            GLOBAL_POOL_SIZE.load(Ordering::Relaxed)
        }
    }
}

impl Drop for OnDemandGlobalLock {
    fn drop(&mut self) {
        // Updates [`GLOBAL_POOL_SIZE`] if the global pool is locked.
        if let Some(guard) = self.guard.as_ref() {
            GLOBAL_POOL_SIZE.store(guard.total_size(), Ordering::Relaxed);
        }
    }
}

#[cfg(ktest)]
pub(crate) mod test {
    use core::alloc::Layout;

    use ostd::{
        mm,
        sync::{LocalIrqDisabled, SpinLock},
    };

    use super::OnDemandGlobalLock;
    use crate::{chunk, set::BuddySet};

    static TEST_GLOBAL_POOL: SpinLock<BuddySet<{ super::MAX_BUDDY_ORDER }>, LocalIrqDisabled> =
        SpinLock::new(BuddySet::new_empty());

    pub(crate) struct TestGlobalPool {
        pub(super) lock: OnDemandGlobalLock,
    }

    impl TestGlobalPool {
        pub(crate) fn new() -> Self {
            Self {
                lock: OnDemandGlobalLock {
                    guard: Some(TEST_GLOBAL_POOL.lock()),
                },
            }
        }

        pub(crate) fn attempt(
            &mut self,
            local: &mut BuddySet<{ super::MAX_LOCAL_BUDDY_ORDER }>,
            layout: Layout,
        ) -> Option<mm::Paddr> {
            super::alloc_from(local, &mut self.lock, layout)
        }

        pub(crate) fn total_size(&mut self) -> usize {
            self.lock.get().total_size()
        }

        pub(crate) fn alloc_chunk(&mut self, order: chunk::BuddyOrder) -> Option<mm::Paddr> {
            self.lock.get().alloc_chunk(order)
        }
    }

    impl Drop for TestGlobalPool {
        fn drop(&mut self) {
            while self.lock.get().alloc_chunk(0).is_some() {}
            // Leave no guard for OnDemandGlobalLock to publish as the real pool's size.
            drop(self.lock.guard.take());
        }
    }
}
