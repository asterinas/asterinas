// SPDX-License-Identifier: MPL-2.0

mod balancing;

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
            add_free_memory_to(
                &mut local_pool,
                guard,
                chunk_addr + layout.size(),
                allocated_size - layout.size(),
            );
        }
    }

    balancing::balance(local_pool.deref_mut());

    LOCAL_POOL_SIZE
        .get_on_cpu(guard.current_cpu())
        .store(local_pool.total_size(), Ordering::Relaxed);

    chunk_addr
}

pub(super) fn dealloc(guard: &DisabledLocalIrqGuard, addr: Paddr, size: usize) {
    let local_pool_cell = LOCAL_POOL.get_with(guard);
    let mut local_pool = local_pool_cell.borrow_mut();

    add_free_memory_to(&mut local_pool, guard, addr, size);
}

pub(super) fn add_free_memory(guard: &DisabledLocalIrqGuard, addr: Paddr, size: usize) {
    let local_pool_cell = LOCAL_POOL.get_with(guard);
    let mut local_pool = local_pool_cell.borrow_mut();

    add_free_memory_to(&mut local_pool, guard, addr, size);
}

fn add_free_memory_to(
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
