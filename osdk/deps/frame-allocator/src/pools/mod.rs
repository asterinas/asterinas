// SPDX-License-Identifier: MPL-2.0

mod balancing;

use core::{
    alloc::Layout,
    cell::RefCell,
    ops::DerefMut,
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd::{
    cpu, cpu_local,
    irq::DisabledLocalIrqGuard,
    mm::Paddr,
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard},
};

use crate::chunk::{self, BuddyOrder};

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

/// Outcome of refilling an empty per-CPU frame cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RefillAlloc {
    Batch(Paddr),
    Single(Paddr),
}

pub(super) fn alloc(guard: &DisabledLocalIrqGuard, layout: Layout) -> Option<Paddr> {
    let local_pool_cell = LOCAL_POOL.get_with(guard);
    let mut local_pool = local_pool_cell.borrow_mut();
    let mut global_pool = OnDemandGlobalLock::new();

    alloc_from(&mut local_pool, &mut global_pool, layout, cpu::num_cpus())
}

pub(super) fn alloc_refill(
    guard: &DisabledLocalIrqGuard,
    batch_layout: Layout,
    single_layout: Layout,
) -> Option<RefillAlloc> {
    let local_pool_cell = LOCAL_POOL.get_with(guard);
    let mut local_pool = local_pool_cell.borrow_mut();
    let mut global_pool = OnDemandGlobalLock::new();

    alloc_refill_from(
        &mut local_pool,
        &mut global_pool,
        batch_layout,
        single_layout,
        cpu::num_cpus(),
    )
}

fn alloc_from(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global_pool: &mut OnDemandGlobalLock,
    layout: Layout,
    nr_cpus: usize,
) -> Option<Paddr> {
    alloc_with_balance(local_pool, global_pool, nr_cpus, |local, global| {
        try_alloc_from_local(local, global, layout)
            .or_else(|| try_alloc_from_global(local, global, layout))
    })
}

fn alloc_refill_from(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global_pool: &mut OnDemandGlobalLock,
    batch_layout: Layout,
    single_layout: Layout,
    nr_cpus: usize,
) -> Option<RefillAlloc> {
    alloc_with_balance(local_pool, global_pool, nr_cpus, |local, global| {
        // Probe `LOCAL_POOL` for both the batch and single segment before locking
        // `GLOBAL_POOL`, and probe both layouts before `balance_for_cpus` can split
        // an intact segment in `GLOBAL_POOL`.
        try_alloc_from_local(local, global, batch_layout)
            .map(RefillAlloc::Batch)
            .or_else(|| try_alloc_from_local(local, global, single_layout).map(RefillAlloc::Single))
            .or_else(|| try_alloc_from_global(local, global, batch_layout).map(RefillAlloc::Batch))
            .or_else(|| {
                try_alloc_from_global(local, global, single_layout).map(RefillAlloc::Single)
            })
    })
}

fn alloc_with_balance<T>(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global_pool: &mut OnDemandGlobalLock,
    nr_cpus: usize,
    try_alloc_fn: impl Fn(&mut BuddySet<MAX_LOCAL_BUDDY_ORDER>, &mut OnDemandGlobalLock) -> Option<T>,
) -> Option<T> {
    if let Some(allocated) = try_alloc_fn(local_pool, global_pool) {
        balancing::balance_for_cpus(local_pool, global_pool, nr_cpus);
        return Some(allocated);
    }

    // Flush `local_pool` into `GLOBAL_POOL` on a miss so buddies split between
    // this CPU and `GLOBAL_POOL` coalesce regardless of watermarks or list depth.
    if balancing::drain_to_global(local_pool, global_pool)
        && let Some(allocated) = try_alloc_fn(local_pool, global_pool)
    {
        balancing::balance_for_cpus(local_pool, global_pool, nr_cpus);
        return Some(allocated);
    }

    None
}

fn layout_order(layout: Layout) -> BuddyOrder {
    chunk::greater_order_of(layout.size()).max(chunk::greater_order_of(layout.align()))
}

fn try_alloc_from_local(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global_pool: &mut OnDemandGlobalLock,
    layout: Layout,
) -> Option<Paddr> {
    let order = layout_order(layout);
    if order >= MAX_LOCAL_BUDDY_ORDER {
        return None;
    }
    let chunk_addr = local_pool.alloc_chunk(order)?;
    return_unused_tail(local_pool, global_pool, chunk_addr, order, layout.size());
    Some(chunk_addr)
}

fn try_alloc_from_global(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global_pool: &mut OnDemandGlobalLock,
    layout: Layout,
) -> Option<Paddr> {
    // TODO: On memory pressure the global pool may be not enough. We may need
    // to merge all buddy chunks from the local pools to the global pool and
    // try again.
    let order = layout_order(layout);
    let chunk_addr = global_pool.get().alloc_chunk(order)?;
    return_unused_tail(local_pool, global_pool, chunk_addr, order, layout.size());
    Some(chunk_addr)
}

fn return_unused_tail(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global_pool: &mut OnDemandGlobalLock,
    chunk_addr: Paddr,
    order: BuddyOrder,
    requested_size: usize,
) {
    // If the buddy chunk is larger than `requested_size`, return the unused
    // tail to the free lists.
    let allocated_size = chunk::size_of_order(order);
    if allocated_size > requested_size {
        do_dealloc(
            local_pool,
            global_pool,
            [(chunk_addr + requested_size, allocated_size - requested_size)].into_iter(),
        );
    }
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

    chunk::split_to_chunks(addr, size).for_each(|(addr, order)| {
        global_pool.get().insert_chunk(addr, order);
    });
}

fn do_dealloc(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global_pool: &mut OnDemandGlobalLock,
    segments: impl Iterator<Item = (Paddr, usize)>,
) {
    segments.for_each(|(addr, size)| {
        chunk::split_to_chunks(addr, size).for_each(|(addr, order)| {
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
    use core::{alloc::Layout, marker::PhantomData};

    use ostd::{
        mm,
        sync::{LocalIrqDisabled, SpinLock},
    };

    use super::{OnDemandGlobalLock, RefillAlloc, balancing};
    use crate::{chunk, set::BuddySet, test::MockMemoryRegion};

    static TEST_GLOBAL_POOL: SpinLock<BuddySet<{ super::MAX_BUDDY_ORDER }>, LocalIrqDisabled> =
        SpinLock::new(BuddySet::new_empty());

    pub(crate) struct TestGlobalPool<'a> {
        lock: OnDemandGlobalLock,
        _region: PhantomData<&'a MockMemoryRegion>,
    }

    impl<'a> TestGlobalPool<'a> {
        pub(crate) fn new(_region: &'a MockMemoryRegion) -> Self {
            Self {
                lock: OnDemandGlobalLock {
                    guard: Some(TEST_GLOBAL_POOL.lock()),
                },
                _region: PhantomData,
            }
        }

        pub(crate) fn insert_chunk(&mut self, addr: mm::Paddr, order: chunk::BuddyOrder) {
            self.lock.get().insert_chunk(addr, order);
        }

        pub(super) fn alloc_from_pools(
            &mut self,
            local: &mut BuddySet<{ super::MAX_LOCAL_BUDDY_ORDER }>,
            layout: Layout,
            nr_cpus: usize,
        ) -> Option<mm::Paddr> {
            super::alloc_from(local, &mut self.lock, layout, nr_cpus)
        }

        pub(crate) fn alloc_refill_from_pools(
            &mut self,
            local: &mut BuddySet<{ super::MAX_LOCAL_BUDDY_ORDER }>,
            batch_layout: Layout,
            single_layout: Layout,
            nr_cpus: usize,
        ) -> Option<RefillAlloc> {
            super::alloc_refill_from(local, &mut self.lock, batch_layout, single_layout, nr_cpus)
        }

        pub(super) fn balance_for_cpus(
            &mut self,
            local: &mut BuddySet<{ super::MAX_LOCAL_BUDDY_ORDER }>,
            nr_cpus: usize,
        ) -> bool {
            balancing::balance_for_cpus(local, &mut self.lock, nr_cpus)
        }

        pub(super) fn balance_with_snapshot(
            &mut self,
            local: &mut BuddySet<{ super::MAX_LOCAL_BUDDY_ORDER }>,
            snapshot_size: usize,
            nr_cpus: usize,
        ) -> bool {
            let global_set = &mut **self.lock.get();
            balancing::balance_with_snapshot(local, snapshot_size, nr_cpus, || global_set)
        }

        pub(crate) fn total_size(&mut self) -> usize {
            self.lock.get().total_size()
        }

        pub(crate) fn alloc_chunk(&mut self, order: chunk::BuddyOrder) -> Option<mm::Paddr> {
            self.lock.get().alloc_chunk(order)
        }

        pub(crate) fn nr_alloc_attempts(&mut self) -> usize {
            self.lock.get().nr_alloc_attempts()
        }
    }

    impl Drop for TestGlobalPool<'_> {
        fn drop(&mut self) {
            while self.lock.get().alloc_chunk(0).is_some() {}
            self.lock.get().reset_alloc_attempts();
            // Clear `guard` so `OnDemandGlobalLock::drop` does not overwrite `GLOBAL_POOL_SIZE`.
            drop(self.lock.guard.take());
        }
    }
}
