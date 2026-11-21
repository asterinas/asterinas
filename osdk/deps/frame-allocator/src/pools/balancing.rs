// SPDX-License-Identifier: MPL-2.0

//! Controlling the balancing between CPU-local free pools and the global free pool.

use ostd::{cpu::num_cpus, mm};

use super::{BuddyOrder, BuddySet, MAX_LOCAL_BUDDY_ORDER, OnDemandGlobalLock, lesser_order_of};

use crate::chunk::split_to_order;

/// Caps the expected size of each CPU-local free pool so one CPU cannot
/// accumulate hundreds of megabytes of isolated single pages.
const CACHE_MAX_EXPECTED_SIZE: usize = 2 * 1024 * 1024;

/// Controls the expected size of cache for each CPU-local free pool.
///
/// The expected size will be the size of `GLOBAL_POOL` divided by the number
/// of the CPUs, and then divided by this constant.
const CACHE_EXPECTED_PORTION: usize = 2;

/// Returns the expected size of cache for each CPU-local free pool.
///
/// It depends on the size of the global free pool, up to
/// [`CACHE_MAX_EXPECTED_SIZE`].
fn cache_expected_size(global_size: usize) -> usize {
    (global_size / num_cpus() / CACHE_EXPECTED_PORTION).min(CACHE_MAX_EXPECTED_SIZE)
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
pub(super) fn balance(
    local: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global: &mut OnDemandGlobalLock,
) {
    let global_size = global.get_global_size();

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
        if expected_removal < mm::PAGE_SIZE {
            return;
        }
        let lesser_order = lesser_order_of(expected_removal);

        balance_to(local, &mut *global.get(), lesser_order);
    } else if local_size < minimal_local_size {
        // Move global frames to the local pool.
        if global_size == 0 {
            return;
        }

        let expected_allocation = expected_local_size - local_size;
        if expected_allocation < mm::PAGE_SIZE {
            return;
        }
        let lesser_order = lesser_order_of(expected_allocation);

        balance_to(&mut *global.get(), local, lesser_order);
    }
}

/// Balances from `a` to `b`.
///
/// Returns whether the full requested size was moved.
fn balance_to<const MAX_ORDER1: BuddyOrder, const MAX_ORDER2: BuddyOrder>(
    a: &mut BuddySet<MAX_ORDER1>,
    b: &mut BuddySet<MAX_ORDER2>,
    order: BuddyOrder,
) -> bool {
    let allocated_from_a = a.alloc_chunk(order);

    if let Some(addr) = allocated_from_a {
        if order >= MAX_ORDER2 {
            let inserted_order = MAX_ORDER2 - 1;
            split_to_order(addr, order, inserted_order).for_each(|addr| {
                b.insert_chunk(addr, inserted_order);
            });
        } else {
            b.insert_chunk(addr, order);
        }

        true
    } else {
        // An incomplete first half exhausts all transferable source chunks,
        // so the second half would only probe the same empty orders.
        order > 0 && balance_to(a, b, order - 1) && balance_to(a, b, order - 1)
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{cpu, irq, mm, prelude::ktest};

    use crate::{
        chunk, pools,
        test::{MockMemoryRegion, TestBuddySet},
    };

    #[ktest]
    fn subpage_balancing_keeps_pools_unchanged() {
        // Regression for #3956. With one global page per CPU the local target is
        // half a page, whether the local pool needs to import or export frames.
        let global_size = cpu::num_cpus() * mm::PAGE_SIZE;
        let region_order = chunk::greater_order_of(global_size + mm::PAGE_SIZE);
        let region = MockMemoryRegion::alloc(chunk::size_of_order(region_order));
        assert_eq!(super::cache_expected_size(global_size), mm::PAGE_SIZE / 2);

        for local_pages in [0, 1] {
            let _irq = irq::disable_local();
            let mut global = pools::test::TestGlobalPool::new();
            let mut local = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty();
            if local_pages == 1 {
                local.insert_chunk(region.paddr(), 0);
            }
            for (addr, order) in chunk::split_to_chunks(region.paddr() + mm::PAGE_SIZE, global_size)
            {
                global.lock.get().insert_chunk(addr, order);
            }

            super::balance(&mut local, &mut global.lock);

            assert_eq!(local.total_size(), local_pages * mm::PAGE_SIZE);
            assert_eq!(global.total_size(), global_size);
        }
    }

    #[ktest]
    fn partial_transfer_stops_after_exhaustion() {
        const TARGET_ORDER: usize = 6;
        let region = MockMemoryRegion::alloc(8 * mm::PAGE_SIZE);
        let base = region.paddr();
        let mut source = TestBuddySet::<{ TARGET_ORDER + 1 }>::new_empty();
        let mut destination = TestBuddySet::<2>::new_empty();
        source.insert_chunk(base, 2);
        source.insert_chunk(base + 4 * mm::PAGE_SIZE, 1);
        source.insert_chunk(base + 7 * mm::PAGE_SIZE, 0);

        let complete = super::balance_to(&mut source, &mut destination, TARGET_ORDER);
        let attempts = source.alloc_attempts();

        assert!(!complete);
        assert_eq!(source.total_size(), 0);
        assert_eq!(source.alloc_chunk(0), None);
        assert_eq!(destination.total_size(), 7 * mm::PAGE_SIZE);
        let mut chunks = core::array::from_fn::<_, 3, _>(|_| destination.alloc_chunk(1).unwrap());
        chunks.sort_unstable();
        assert_eq!(
            chunks,
            [base, base + 2 * mm::PAGE_SIZE, base + 4 * mm::PAGE_SIZE]
        );
        assert_eq!(destination.alloc_chunk(0), Some(base + 7 * mm::PAGE_SIZE));
        assert_eq!(destination.total_size(), 0);

        // Regression for #3956. The source runs out after transferring all
        // available chunks down to order 0 without repeating the second half.
        assert!(
            attempts <= 2 * TARGET_ORDER,
            "balance took {attempts} source probes"
        );
    }

    #[ktest]
    fn cap_and_order0_export_drains_isolated_pages() {
        assert_eq!(
            super::cache_expected_size(64 * super::CACHE_MAX_EXPECTED_SIZE),
            super::CACHE_MAX_EXPECTED_SIZE
        );

        let region = MockMemoryRegion::alloc(8 * mm::PAGE_SIZE);
        let base = region.paddr();
        let _irq = irq::disable_local();
        let mut local = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty();
        let mut global = pools::test::TestGlobalPool::new();

        for i in 0..4 {
            local.insert_chunk(base + 2 * i * mm::PAGE_SIZE, 0);
        }

        super::balance(&mut local, &mut global.lock);

        assert_eq!(local.total_size(), 0);
        assert_eq!(global.total_size(), 4 * mm::PAGE_SIZE);
    }

    #[ktest]
    fn order0_import_coalesces_buddy_beyond_32_pages() {
        const DUMMY_PAGES: usize = 1024;
        const BUDGET_PAGES: usize = 32;
        let region = MockMemoryRegion::alloc(chunk::size_of_order(12));
        let base = region.paddr();
        let _irq = irq::disable_local();
        let mut local = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty();
        let mut global = pools::test::TestGlobalPool::new();

        local.insert_chunk(base, 0);
        for index in 1..=DUMMY_PAGES - BUDGET_PAGES {
            global
                .lock
                .get()
                .insert_chunk(base + 2 * index * mm::PAGE_SIZE, 0);
        }
        global.lock.get().insert_chunk(base + mm::PAGE_SIZE, 0);
        for index in DUMMY_PAGES - BUDGET_PAGES + 1..=DUMMY_PAGES {
            global
                .lock
                .get()
                .insert_chunk(base + 2 * index * mm::PAGE_SIZE, 0);
        }

        let bulk = core::alloc::Layout::from_size_align(8 * mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();
        let requested =
            core::alloc::Layout::from_size_align(2 * mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();

        assert!(global.attempt(&mut local, bulk).is_none());
        assert_eq!(global.attempt(&mut local, requested), Some(base));
    }
}
