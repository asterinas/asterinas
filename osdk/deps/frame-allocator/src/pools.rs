// SPDX-License-Identifier: MPL-2.0

use core::{
    alloc::Layout,
    cell::RefCell,
    ops::DerefMut,
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd::{
    cpu::PinCurrentCpu,
    cpu_local,
    mm::Paddr,
    sync::{LocalIrqDisabled, SpinLock},
    trap::DisabledLocalIrqGuard,
};

use crate::chunk::{greater_order_of, lesser_order_of, max_order_from, size_of_order, BuddyOrder};

use super::set::BuddySet;

/// The global free buddies.
static GLOBAL_POOL: SpinLock<BuddySet<MAX_BUDDY_ORDER>, LocalIrqDisabled> =
    SpinLock::new(BuddySet::new_empty());
static GLOBAL_POOL_SIZE: AtomicUsize = AtomicUsize::new(0);

// CPU-local free buddies.
cpu_local! {
    static LOCAL_POOL: RefCell<BuddySet<MAX_LOCAL_BUDDY_ORDER>> = RefCell::new(BuddySet::new_empty());
    static LOCAL_POOL_SIZE: AtomicUsize = AtomicUsize::new(0);
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

    let size_order = greater_order_of(layout.size());
    let align_order = greater_order_of(layout.align());

    let order = size_order.max(align_order);
    let mut chunk_addr = None;

    if order < MAX_LOCAL_BUDDY_ORDER {
        chunk_addr = local_pool.alloc_chunk(order);
    }

    // Fall back to the global free lists if the local free lists are empty.
    if chunk_addr.is_none() {
        chunk_addr = alloc_from_global_pool(order);
    }
    // TODO: On memory pressure the global pool may be not enough. We may need
    // to merge all buddy chunks from the local pools to the global pool and
    // try again.

    // If the alignment order is larger than the size order, we need to split
    // the chunk and return the rest part back to the free lists.
    let allocated_size = size_of_order(order);
    if allocated_size > layout.size() {
        if let Some(chunk_addr) = chunk_addr {
            dealloc_in(
                &mut local_pool,
                guard,
                chunk_addr + layout.size(),
                allocated_size - layout.size(),
            );
        }
    } else {
        balancing::balance(local_pool.deref_mut());
    }

    LOCAL_POOL_SIZE
        .get_on_cpu(guard.current_cpu())
        .store(local_pool.total_size(), Ordering::Relaxed);

    chunk_addr
}

pub(super) fn add_free_memory(guard: &DisabledLocalIrqGuard, addr: Paddr, size: usize) {
    let local_pool_cell = LOCAL_POOL.get_with(guard);
    let mut local_pool = local_pool_cell.borrow_mut();

    dealloc_in(&mut local_pool, guard, addr, size);
}

fn dealloc_in(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    guard: &DisabledLocalIrqGuard,
    mut addr: Paddr,
    mut size: usize,
) {
    // Split the range into chunks and return them to the local free lists
    // respectively.
    while size > 0 {
        let next_chunk_order = max_order_from(addr).min(lesser_order_of(size));

        if next_chunk_order >= MAX_LOCAL_BUDDY_ORDER {
            dealloc_to_global_pool(addr, next_chunk_order);
        } else {
            local_pool.insert_chunk(addr, next_chunk_order);
        }

        size -= size_of_order(next_chunk_order);
        addr += size_of_order(next_chunk_order);
    }

    balancing::balance(local_pool);
    LOCAL_POOL_SIZE
        .get_on_cpu(guard.current_cpu())
        .store(local_pool.total_size(), Ordering::Relaxed);
}

fn alloc_from_global_pool(order: BuddyOrder) -> Option<Paddr> {
    let mut lock_guard = GLOBAL_POOL.lock();
    let res = lock_guard.alloc_chunk(order);
    GLOBAL_POOL_SIZE.store(lock_guard.total_size(), Ordering::Relaxed);
    res
}

fn dealloc_to_global_pool(addr: Paddr, order: BuddyOrder) {
    let mut lock_guard = GLOBAL_POOL.lock();
    lock_guard.insert_chunk(addr, order);
    GLOBAL_POOL_SIZE.store(lock_guard.total_size(), Ordering::Relaxed);
}

pub mod balancing {
    //! Controlling the balancing between CPU-local free pools and the global free pool.

    use core::sync::atomic::Ordering;

    use ostd::cpu::num_cpus;

    use super::{
        lesser_order_of, BuddyOrder, BuddySet, GLOBAL_POOL, GLOBAL_POOL_SIZE, MAX_LOCAL_BUDDY_ORDER,
    };

    use crate::chunk::size_of_order;

    /// Controls the expected size of cache for each CPU-local free pool.
    ///
    /// The expected size will be the size of `GLOBAL_POOL` divided by the number
    /// of the CPUs, and then divided by this constant.
    const CACHE_EXPECTED_PORTION: usize = 2;

    /// Returns the expected size of cache for each CPU-local free pool.
    ///
    /// It depends on the size of the global free pool.
    fn cache_expected_size(global_size: usize) -> usize {
        global_size / num_cpus() / CACHE_EXPECTED_PORTION
    }

    /// Controls the minimal size of cache for each CPU-local free pool.
    ///
    /// The minimal will be the expected size divided by this constant.
    const CACHE_MINIMAL_PORTION: usize = 8;

    /// Returns the minimal size of cache for each CPU-local free pool.
    ///
    /// It depends on the size of the global free pool.
    fn cache_minimal_size(global_size: usize) -> usize {
        cache_expected_size(global_size) / CACHE_MINIMAL_PORTION
    }

    /// Controls the maximal size of cache for each CPU-local free pool.
    ///
    /// The maximal will be the expected size multiplied by this constant.
    const CACHE_MAXIMAL_MULTIPLIER: usize = 2;

    /// Returns the maximal size of cache for each CPU-local free pool.
    ///
    /// It depends on the size of the global free pool.
    fn cache_maximal_size(global_size: usize) -> usize {
        cache_expected_size(global_size) * CACHE_MAXIMAL_MULTIPLIER
    }

    /// Balances a local cache and the global free pool.
    pub fn balance(local: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>) {
        let global_size = GLOBAL_POOL_SIZE.load(Ordering::Relaxed);

        let minimal_local_size = cache_minimal_size(global_size);
        let expected_local_size = cache_expected_size(global_size);
        let maximal_local_size = cache_maximal_size(global_size);

        let local_size = local.total_size();

        if local_size >= maximal_local_size {
            // Move local frames to the global pool.
            if local_size == 0 {
                return;
            }

            let expected_removal = local_size - expected_local_size;
            let lesser_order = lesser_order_of(expected_removal);
            let mut global_pool_lock = GLOBAL_POOL.lock();

            balance_to(local, &mut *global_pool_lock, lesser_order);

            GLOBAL_POOL_SIZE.store(global_pool_lock.total_size(), Ordering::Relaxed);
        } else if local_size < minimal_local_size {
            // Move global frames to the local pool.
            if global_size == 0 {
                return;
            }

            let expected_allocation = expected_local_size - local_size;
            let lesser_order = lesser_order_of(expected_allocation);
            let mut global_pool_lock = GLOBAL_POOL.lock();

            balance_to(&mut *global_pool_lock, local, lesser_order);

            GLOBAL_POOL_SIZE.store(global_pool_lock.total_size(), Ordering::Relaxed);
        }
    }

    /// Balances from `a` to `b`.
    fn balance_to<const MAX_ORDER1: BuddyOrder, const MAX_ORDER2: BuddyOrder>(
        a: &mut BuddySet<MAX_ORDER1>,
        b: &mut BuddySet<MAX_ORDER2>,
        order: BuddyOrder,
    ) {
        let allocated_from_a = a.alloc_chunk(order);

        if let Some(addr) = allocated_from_a {
            if order >= MAX_ORDER2 {
                let inserted_order = MAX_ORDER2 - 1;
                for i in 0..(1 << (order - inserted_order)) as usize {
                    let split_addr = addr + size_of_order(inserted_order) * i;
                    b.insert_chunk(split_addr, inserted_order);
                }
            } else {
                b.insert_chunk(addr, order);
            }
        } else {
            // Maybe the chunk size is too large.
            // Try to reduce the order and balance again.
            if order > 1 {
                balance_to(a, b, order - 1);
                balance_to(a, b, order - 1);
            }
        }
    }
}
