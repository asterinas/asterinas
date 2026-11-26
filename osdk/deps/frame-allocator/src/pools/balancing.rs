// SPDX-License-Identifier: MPL-2.0

//! Controlling the balancing between CPU-local free pools and the global free pool.

use ostd::{cpu, mm};

use super::{BuddyOrder, BuddySet, MAX_LOCAL_BUDDY_ORDER, OnDemandGlobalLock};
use crate::chunk;

/// Controls the expected size of cache for each CPU-local free pool.
///
/// The expected size will be the size of `GLOBAL_POOL` divided by the number
/// of the CPUs, and then divided by this constant.
const CACHE_EXPECTED_PORTION: usize = 2;

/// Returns the expected size of cache for each CPU-local free pool.
///
/// It depends on the size of the global free pool.
fn cache_expected_size(global_size: usize, nr_cpus: usize) -> usize {
    global_size / nr_cpus / CACHE_EXPECTED_PORTION
}

/// Controls the minimal size of cache for each CPU-local free pool.
///
/// The minimal will be the expected size divided by this constant.
const CACHE_MINIMAL_PORTION: usize = 8;

/// Returns the minimal size of cache for each CPU-local free pool.
///
/// It depends on the size of the global free pool.
fn cache_minimal_size(global_size: usize, nr_cpus: usize) -> usize {
    cache_expected_size(global_size, nr_cpus) / CACHE_MINIMAL_PORTION
}

/// Controls the maximal size of cache for each CPU-local free pool.
///
/// The maximal will be the expected size multiplied by this constant.
const CACHE_MAXIMAL_MULTIPLIER: usize = 2;

/// Returns the maximal size of cache for each CPU-local free pool.
///
/// It depends on the size of the global free pool.
fn cache_maximal_size(global_size: usize, nr_cpus: usize) -> usize {
    cache_expected_size(global_size, nr_cpus) * CACHE_MAXIMAL_MULTIPLIER
}

/// Balances a local cache and the global free pool.
pub(super) fn balance(
    local: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global: &mut OnDemandGlobalLock,
) {
    balance_for_cpus(local, global, cpu::num_cpus());
}

/// Balances a local cache and the global free pool sized for `nr_cpus` CPUs.
pub(super) fn balance_for_cpus(
    local: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global: &mut OnDemandGlobalLock,
    nr_cpus: usize,
) {
    let snapshot_size = global.get_global_size();
    balance_with_snapshot(local, snapshot_size, nr_cpus, || &mut *global.get());
}

fn should_lock_global_for_balance(local_size: usize, global_size: usize, nr_cpus: usize) -> bool {
    let minimal_local_size = cache_minimal_size(global_size, nr_cpus);
    let expected_local_size = cache_expected_size(global_size, nr_cpus);
    let maximal_local_size = cache_maximal_size(global_size, nr_cpus);

    // `expected_local_size` can split a page across CPUs, whereas pools
    // transfer whole pages and `lesser_order_of` requires `>= PAGE_SIZE`.
    if local_size >= maximal_local_size {
        local_size - expected_local_size >= mm::PAGE_SIZE
    } else if local_size < minimal_local_size {
        global_size > 0 && expected_local_size - local_size >= mm::PAGE_SIZE
    } else {
        false
    }
}

fn balance_with_snapshot<'a>(
    local: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    snapshot_size: usize,
    nr_cpus: usize,
    lock_global_fn: impl FnOnce() -> &'a mut BuddySet<{ super::MAX_BUDDY_ORDER }>,
) {
    let local_size = local.total_size();
    if !should_lock_global_for_balance(local_size, snapshot_size, nr_cpus) {
        return;
    }

    // Recheck watermarks against the locked pool size because `snapshot_size`
    // can be stale when `GLOBAL_POOL` was not yet locked.
    let global_set = lock_global_fn();
    let global_size = global_set.total_size();
    let minimal_local_size = cache_minimal_size(global_size, nr_cpus);
    let expected_local_size = cache_expected_size(global_size, nr_cpus);
    let maximal_local_size = cache_maximal_size(global_size, nr_cpus);

    if local_size >= maximal_local_size {
        let expected_removal_bytes = local_size - expected_local_size;
        if expected_removal_bytes < mm::PAGE_SIZE {
            return;
        }
        let lesser_order = chunk::lesser_order_of(expected_removal_bytes);

        balance_to(local, global_set, lesser_order);
    } else if local_size < minimal_local_size {
        if global_size == 0 {
            return;
        }

        let expected_allocation_bytes = expected_local_size - local_size;
        if expected_allocation_bytes < mm::PAGE_SIZE {
            return;
        }
        let lesser_order = chunk::lesser_order_of(expected_allocation_bytes);

        balance_to(global_set, local, lesser_order);
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
            chunk::split_to_order(addr, order, inserted_order).for_each(|addr| {
                b.insert_chunk(addr, inserted_order);
            });
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

#[cfg(ktest)]
mod test {
    use ostd::{irq, mm, prelude::ktest};

    use crate::{
        chunk, pools,
        test::{MockMemoryRegion, TestBuddySet},
    };

    #[ktest]
    fn subpage_balancing_keeps_pools_unchanged() {
        // One global page per CPU sets `expected_local_size` to `PAGE_SIZE / 2`,
        // exercising sub-page deltas on both import (`0` local pages) and export
        // (`1` local page) across single-core and SMP configurations.
        for nr_cpus in [1, 4, 64] {
            let global_size = nr_cpus * mm::PAGE_SIZE;
            let region_order = chunk::greater_order_of(global_size + mm::PAGE_SIZE);
            let region = MockMemoryRegion::alloc(chunk::size_of_order(region_order));
            assert_eq!(
                super::cache_expected_size(global_size, nr_cpus),
                mm::PAGE_SIZE / 2
            );

            for local_pages in [0, 1] {
                let _irq = irq::disable_local();
                let mut global = pools::test::TestGlobalPool::new(&region);
                let mut local =
                    TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
                if local_pages == 1 {
                    local.insert_chunk(region.paddr(), 0);
                }
                for (addr, order) in
                    chunk::split_to_chunks(region.paddr() + mm::PAGE_SIZE, global_size)
                {
                    global.insert_chunk(addr, order);
                }

                global.balance_for_cpus(&mut local, nr_cpus);

                assert_eq!(local.total_size(), local_pages * mm::PAGE_SIZE);
                assert_eq!(global.total_size(), global_size);
            }
        }
    }
}
