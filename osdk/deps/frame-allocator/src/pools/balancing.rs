// SPDX-License-Identifier: MPL-2.0

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
