// SPDX-License-Identifier: MPL-2.0

//! Controlling the balancing between CPU-local free pools and the global free pool.

use ostd::{cpu, mm};

use super::{BuddyOrder, BuddySet, MAX_LOCAL_BUDDY_ORDER, OnDemandGlobalLock};
use crate::chunk;

/// The maximum expected size in bytes of each CPU-local free pool.
///
/// Capping the per-CPU target at 2 MiB lowers the export watermark so remote
/// `LOCAL_POOL`s do not retain large amounts of isolated `order 0` frames, and
/// bounds each import target while local IRQs are disabled.
const CACHE_MAX_EXPECTED_BYTES: usize = 2 * 1024 * 1024;

/// Controls the expected size of cache for each CPU-local free pool.
///
/// The expected size will be the size of `GLOBAL_POOL` divided by the number
/// of the CPUs, and then divided by this constant, up to
/// [`CACHE_MAX_EXPECTED_BYTES`].
const CACHE_EXPECTED_PORTION: usize = 2;

/// Returns the expected size of cache for each CPU-local free pool.
///
/// It depends on the size of the global free pool, up to
/// [`CACHE_MAX_EXPECTED_BYTES`].
fn cache_expected_size(global_size: usize, nr_cpus: usize) -> usize {
    (global_size / nr_cpus / CACHE_EXPECTED_PORTION).min(CACHE_MAX_EXPECTED_BYTES)
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
///
/// Returns `true` if at least one chunk was transferred between `local` and
/// `global`.
pub(super) fn balance_for_cpus(
    local: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global: &mut OnDemandGlobalLock,
    nr_cpus: usize,
) -> bool {
    let snapshot_size = global.get_global_size();
    balance_with_snapshot(local, snapshot_size, nr_cpus, || &mut *global.get())
}

/// Moves all chunks from `local` into `global` so physical buddies split across
/// pools can coalesce in `GLOBAL_POOL`.
pub(super) fn drain_to_global(
    local: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    global: &mut OnDemandGlobalLock,
) -> bool {
    let local_pages = local.total_size() / mm::PAGE_SIZE;
    if local_pages == 0 {
        return false;
    }
    let moved_pages = transfer_existing_chunks(local, &mut *global.get(), local_pages, None);
    debug_assert_eq!(local.total_size(), 0);
    moved_pages > 0
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

/// Balances a local cache against `GLOBAL_POOL` after checking `snapshot_size`.
pub(super) fn balance_with_snapshot<'a>(
    local: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    snapshot_size: usize,
    nr_cpus: usize,
    lock_global_fn: impl FnOnce() -> &'a mut BuddySet<{ super::MAX_BUDDY_ORDER }>,
) -> bool {
    let local_size = local.total_size();
    if !should_lock_global_for_balance(local_size, snapshot_size, nr_cpus) {
        return false;
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
            return false;
        }
        let mut did_move = transfer_existing_chunks(
            local,
            global_set,
            expected_removal_bytes / mm::PAGE_SIZE,
            None,
        ) > 0;
        if local.total_size() >= maximal_local_size {
            let remaining_removal_bytes = local.total_size() - expected_local_size;
            if remaining_removal_bytes >= mm::PAGE_SIZE
                && let Some((addr, chunk_order)) = local
                    .pop_smallest_chunk_at_least(chunk::lesser_order_of(remaining_removal_bytes))
            {
                // Move an oversize local chunk whole rather than splitting it
                // across pools just to land on `expected_local_size`.
                insert_transferred_chunk(global_set, addr, chunk_order);
                did_move = true;
            }
        }
        did_move
    } else if local_size < minimal_local_size {
        if global_size == 0 {
            return false;
        }

        let expected_allocation_bytes = expected_local_size - local_size;
        if expected_allocation_bytes < mm::PAGE_SIZE {
            return false;
        }
        // Prefer chunks that already fit within the page budget, stopping once
        // `local` reaches `minimal_local_size` before taking another multi-page
        // chunk or smaller size class so a post-hit top-up neither splits a
        // larger block nor hoards refill blocks from other CPUs.
        let mut did_move = transfer_existing_chunks(
            global_set,
            local,
            expected_allocation_bytes / mm::PAGE_SIZE,
            Some(minimal_local_size),
        ) > 0;
        if local.total_size() < minimal_local_size {
            let remaining_allocation_bytes = expected_local_size - local.total_size();
            if remaining_allocation_bytes >= mm::PAGE_SIZE {
                did_move |= try_transfer_one_chunk(
                    global_set,
                    local,
                    chunk::lesser_order_of(remaining_allocation_bytes),
                );
            }
        }
        did_move
    } else {
        false
    }
}

fn insert_transferred_chunk<const MAX_ORDER: BuddyOrder>(
    destination: &mut BuddySet<MAX_ORDER>,
    addr: mm::Paddr,
    order: BuddyOrder,
) {
    if order >= MAX_ORDER {
        let inserted_order = MAX_ORDER - 1;
        chunk::split_to_order(addr, order, inserted_order).for_each(|addr| {
            destination.insert_chunk(addr, inserted_order);
        });
    } else {
        destination.insert_chunk(addr, order);
    }
}

/// Moves up to `max_pages` pages from `source` to `destination` without
/// splitting chunks in `source`, returning the number of pages transferred.
///
/// When `low_watermark_bytes` is `Some(limit)`, stops once
/// `destination.total_size()` reaches `limit` before taking another multi-page
/// chunk or descending to a smaller order.
fn transfer_existing_chunks<const MAX_ORDER1: BuddyOrder, const MAX_ORDER2: BuddyOrder>(
    source: &mut BuddySet<MAX_ORDER1>,
    destination: &mut BuddySet<MAX_ORDER2>,
    max_pages: usize,
    low_watermark_bytes: Option<usize>,
) -> usize {
    if max_pages == 0 || MAX_ORDER1 == 0 {
        return 0;
    }

    // Scan from the largest fitting order down to `order 0` once so isolated
    // single frames move between pools and coalesce with buddies freed on
    // other CPUs without revisiting exhausted size classes.
    let mut remaining_pages = max_pages;
    let top_order = (max_pages.ilog2() as BuddyOrder).min(MAX_ORDER1 - 1);
    for candidate_order in (0..=top_order).rev() {
        if low_watermark_bytes.is_some_and(|limit| destination.total_size() >= limit) {
            break;
        }
        let chunk_pages = 1usize << candidate_order;
        while remaining_pages >= chunk_pages {
            if candidate_order > 0
                && low_watermark_bytes.is_some_and(|limit| destination.total_size() >= limit)
            {
                break;
            }
            let Some(addr) = source.alloc_exact_chunk(candidate_order) else {
                break;
            };
            insert_transferred_chunk(destination, addr, candidate_order);
            remaining_pages -= chunk_pages;
        }
        if remaining_pages == 0 {
            break;
        }
    }

    max_pages - remaining_pages
}

/// Transfers one `order` chunk from `source` to `destination`, splitting a
/// larger chunk in `source` when no exact-order chunk remains.
fn try_transfer_one_chunk<const MAX_ORDER1: BuddyOrder, const MAX_ORDER2: BuddyOrder>(
    source: &mut BuddySet<MAX_ORDER1>,
    destination: &mut BuddySet<MAX_ORDER2>,
    order: BuddyOrder,
) -> bool {
    let Some(addr) = source.alloc_chunk(order) else {
        return false;
    };
    insert_transferred_chunk(destination, addr, order);
    true
}

#[cfg(ktest)]
mod test {
    use core::alloc::Layout;

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

                assert!(!global.balance_for_cpus(&mut local, nr_cpus));

                assert_eq!(local.total_size(), local_pages * mm::PAGE_SIZE);
                assert_eq!(global.total_size(), global_size);
            }
        }
    }

    #[ktest]
    fn partial_transfer_stops_after_exhaustion() {
        const TARGET_ORDER: usize = 6;
        const NR_SOURCE_CHUNKS: usize = 3;

        let region = MockMemoryRegion::alloc(8 * mm::PAGE_SIZE);
        let base = region.paddr();
        let mut source = TestBuddySet::<{ TARGET_ORDER + 1 }>::new_empty(&region);
        let mut destination = TestBuddySet::<2>::new_empty(&region);
        source.insert_chunk(base, 2);
        source.insert_chunk(base + 4 * mm::PAGE_SIZE, 1);
        source.insert_chunk(base + 7 * mm::PAGE_SIZE, 0);

        let moved_pages =
            super::transfer_existing_chunks(&mut source, &mut destination, 1 << TARGET_ORDER, None);
        let nr_attempts = source.nr_alloc_attempts();

        assert_eq!(moved_pages, 7);
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

        // Scanning each size class once bounds misses to `TARGET_ORDER + 1`
        // plus one successful pop per transferred chunk.
        assert!(
            nr_attempts <= TARGET_ORDER + 1 + NR_SOURCE_CHUNKS,
            "balance took {nr_attempts} source probes"
        );
    }

    #[ktest]
    fn cap_and_order0_export_drains_isolated_pages() {
        for nr_cpus in [1, 4, 32, 64] {
            let uncapped_above_cap =
                2 * nr_cpus * super::CACHE_EXPECTED_PORTION * super::CACHE_MAX_EXPECTED_BYTES;
            assert_eq!(
                super::cache_expected_size(uncapped_above_cap, nr_cpus),
                super::CACHE_MAX_EXPECTED_BYTES
            );
        }

        // Use a non-power-of-two count (`7` isolated `order 0` pages) so export
        // cannot strand trailing single pages when `GLOBAL_POOL` is empty.
        const NR_ISOLATED_PAGES: usize = 7;
        let region = MockMemoryRegion::alloc(16 * mm::PAGE_SIZE);
        let base = region.paddr();
        let _irq = irq::disable_local();
        let mut local = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
        let mut global = pools::test::TestGlobalPool::new(&region);

        for i in 0..NR_ISOLATED_PAGES {
            local.insert_chunk(base + 2 * i * mm::PAGE_SIZE, 0);
        }

        assert!(global.balance_for_cpus(&mut local, 4));
        assert_eq!(local.total_size(), 0);
        assert_eq!(global.total_size(), NR_ISOLATED_PAGES * mm::PAGE_SIZE);
    }

    #[ktest]
    fn export_moves_oversize_local_chunk_whole() {
        // Reserve an `order 12` (16 MiB) region so `aligned_base`,
        // `aligned_base + 4 MiB`, and `aligned_base + 8 MiB` are aligned for
        // `order 10` and `order 9` chunks without coalescing as buddies.
        let region = MockMemoryRegion::alloc(chunk::size_of_order(12));
        let aligned_base = region.paddr().next_multiple_of(chunk::size_of_order(10));
        let region_end = region.paddr() + chunk::size_of_order(12);
        assert!(
            aligned_base + 2 * chunk::size_of_order(10) + chunk::size_of_order(9) <= region_end
        );

        let _irq = irq::disable_local();
        let mut local = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
        let mut global = pools::test::TestGlobalPool::new(&region);

        // Two non-buddy 2 MiB chunks give `GLOBAL_POOL` 4 MiB (`expected_local_size`
        // = 2 MiB, `maximal_local_size` = 4 MiB on 1 CPU) without any 4 MiB block.
        local.insert_chunk(aligned_base, 10);
        global.insert_chunk(aligned_base + chunk::size_of_order(10), 9);
        global.insert_chunk(aligned_base + 2 * chunk::size_of_order(10), 9);

        assert!(global.balance_for_cpus(&mut local, 1));
        let four_mib = Layout::from_size_align(chunk::size_of_order(10), mm::PAGE_SIZE).unwrap();
        assert_eq!(
            global.alloc_from_pools(&mut local, four_mib, 1),
            Some(aligned_base)
        );
    }

    #[ktest]
    fn miss_coalesces_split_order0_buddy_across_pools() {
        const NR_GLOBAL_PAGES: usize = 600;

        // Exercise buddy depths both within (`64`) and beyond (`550` > `512`
        // pages = `CACHE_MAX_EXPECTED_BYTES`) the per-CPU import target cap.
        for unrelated_prefix_pages in [64, 550] {
            for use_refill_path in [false, true] {
                let region = MockMemoryRegion::alloc(2 * NR_GLOBAL_PAGES * mm::PAGE_SIZE);
                let base = region.paddr();
                let _irq = irq::disable_local();
                let mut local =
                    TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
                let mut global = pools::test::TestGlobalPool::new(&region);

                local.insert_chunk(base, 0);
                for index in 1..NR_GLOBAL_PAGES - unrelated_prefix_pages {
                    global.insert_chunk(base + 2 * index * mm::PAGE_SIZE, 0);
                }
                global.insert_chunk(base + mm::PAGE_SIZE, 0);
                for index in NR_GLOBAL_PAGES - unrelated_prefix_pages..NR_GLOBAL_PAGES {
                    global.insert_chunk(base + 2 * index * mm::PAGE_SIZE, 0);
                }

                let batch = Layout::from_size_align(8 * mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();
                let pair = Layout::from_size_align(2 * mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();

                if use_refill_path {
                    assert_eq!(
                        global.alloc_refill_from_pools(&mut local, batch, pair, 1),
                        Some(pools::RefillAlloc::Single(base))
                    );
                } else {
                    assert_eq!(global.alloc_from_pools(&mut local, pair, 1), Some(base));
                }
            }
        }
    }

    #[ktest]
    fn opportunistic_import_preserves_smaller_global_refill_block() {
        const NR_ISOLATED_PAGES: usize = 72;

        // Exercise both mixed orders (`16` + `8` pages) and repeated multi-page
        // chunks within the same order (`8` + `8` + `8` pages).
        for same_order_blocks in [false, true] {
            for use_refill_path in [false, true] {
                let region = MockMemoryRegion::alloc(chunk::size_of_order(9));
                let aligned_base = region.paddr().next_multiple_of(chunk::size_of_order(4));
                let _irq = irq::disable_local();
                let mut local_a =
                    TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
                let mut local_b =
                    TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
                let mut global = pools::test::TestGlobalPool::new(&region);

                local_a.insert_chunk(aligned_base, 3);
                if same_order_blocks {
                    global.insert_chunk(aligned_base + 16 * mm::PAGE_SIZE, 3);
                    global.insert_chunk(aligned_base + 32 * mm::PAGE_SIZE, 3);
                    global.insert_chunk(aligned_base + 48 * mm::PAGE_SIZE, 3);
                } else {
                    global.insert_chunk(aligned_base + 16 * mm::PAGE_SIZE, 4);
                    global.insert_chunk(aligned_base + 48 * mm::PAGE_SIZE, 3);
                }
                for index in 0..NR_ISOLATED_PAGES {
                    global.insert_chunk(aligned_base + (64 + 2 * index) * mm::PAGE_SIZE, 0);
                }
                assert_eq!(global.total_size(), 96 * mm::PAGE_SIZE);

                let eight_pages =
                    Layout::from_size_align(8 * mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();
                let two_pages = Layout::from_size_align(2 * mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();

                // CPU A satisfies its 8-page request locally. Opportunistic top-up
                // stops after one multi-page block lifts `local_a` above the 3-page
                // low watermark, leaving at least one 8-page block for CPU B.
                assert_eq!(
                    global.alloc_from_pools(&mut local_a, eight_pages, 2),
                    Some(aligned_base)
                );
                let expected_local_a_pages = if same_order_blocks { 8 } else { 16 };
                assert_eq!(local_a.total_size(), expected_local_a_pages * mm::PAGE_SIZE);

                if use_refill_path {
                    assert!(matches!(
                        global.alloc_refill_from_pools(&mut local_b, eight_pages, two_pages, 2),
                        Some(pools::RefillAlloc::Batch(_))
                    ));
                } else {
                    assert!(
                        global
                            .alloc_from_pools(&mut local_b, two_pages, 2)
                            .is_some()
                    );
                }
            }
        }
    }

    #[ktest]
    fn failed_alloc_leaves_global_blocks_for_other_cpus() {
        const NR_ISOLATED_PAGES: usize = 24;

        let region = MockMemoryRegion::alloc(chunk::size_of_order(7));
        let aligned_base = region.paddr().next_multiple_of(chunk::size_of_order(4));
        let eight_page_addr = aligned_base + 16 * mm::PAGE_SIZE;

        let _irq = irq::disable_local();
        let mut local_a = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
        let mut local_b = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
        let mut global = pools::test::TestGlobalPool::new(&region);

        local_a.insert_chunk(aligned_base, 0);
        global.insert_chunk(eight_page_addr, 3);
        for index in 0..NR_ISOLATED_PAGES {
            global.insert_chunk(aligned_base + (32 + 2 * index) * mm::PAGE_SIZE, 0);
        }
        assert_eq!(global.total_size(), 32 * mm::PAGE_SIZE);

        let sixteen_pages = Layout::from_size_align(16 * mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();
        let eight_pages = Layout::from_size_align(8 * mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();

        assert_eq!(
            global.alloc_from_pools(&mut local_a, sixteen_pages, 2),
            None
        );
        assert_eq!(
            global.alloc_from_pools(&mut local_b, eight_pages, 2),
            Some(eight_page_addr)
        );
    }

    #[ktest]
    fn miss_recovers_contiguous_block_split_by_capped_topup() {
        let region = MockMemoryRegion::alloc(chunk::size_of_order(14));
        let aligned_base = region.paddr().next_multiple_of(chunk::size_of_order(12));
        let eight_mib_addr = aligned_base;
        let sixteen_mib_addr = aligned_base + chunk::size_of_order(12);
        let local_page_addr = aligned_base + 2 * chunk::size_of_order(12);

        let _irq = irq::disable_local();
        let mut local = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
        let mut global = pools::test::TestGlobalPool::new(&region);

        local.insert_chunk(local_page_addr, 0);
        global.insert_chunk(eight_mib_addr, 11);
        global.insert_chunk(sixteen_mib_addr, 12);

        let one_page = Layout::from_size_align(mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();
        let eight_mib = Layout::from_size_align(chunk::size_of_order(11), mm::PAGE_SIZE).unwrap();
        let sixteen_mib = Layout::from_size_align(chunk::size_of_order(12), mm::PAGE_SIZE).unwrap();

        // Emptying `local` triggers a capped 2 MiB top-up that splits `eight_mib_addr`
        // across `local` (2 MiB) and `global` (6 MiB). After `sixteen_mib` consumes
        // the 16 MiB block, `eight_mib` flushes `local` on miss and reassembles it.
        assert_eq!(
            global.alloc_from_pools(&mut local, one_page, 1),
            Some(local_page_addr)
        );
        assert_eq!(local.total_size(), super::CACHE_MAX_EXPECTED_BYTES);
        assert_eq!(
            global.alloc_from_pools(&mut local, sixteen_mib, 1),
            Some(sixteen_mib_addr)
        );
        assert_eq!(
            global.alloc_from_pools(&mut local, eight_mib, 1),
            Some(eight_mib_addr)
        );
        assert_eq!(local.total_size() + global.total_size(), 0);
    }

    #[ktest]
    fn import_prefers_small_chunks_before_splitting_large_block() {
        const LARGE_ORDER: usize = 4;
        const LARGE_PAGES: usize = 1 << LARGE_ORDER;

        let region_order = chunk::greater_order_of(4 * LARGE_PAGES * mm::PAGE_SIZE);
        let region = MockMemoryRegion::alloc(chunk::size_of_order(region_order));
        let aligned_base = region
            .paddr()
            .next_multiple_of(chunk::size_of_order(LARGE_ORDER));
        let _irq = irq::disable_local();
        let mut local = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
        let mut global = pools::test::TestGlobalPool::new(&region);

        // Insert one `order 4` (16-page) contiguous block and 16 isolated
        // `order 0` pages into `global`. With `nr_cpus == 2`, the one-page
        // allocation leaves 31 global pages (`expected_local_size` = 7 pages,
        // `minimal_local_size` = 3968 bytes), importing 7 single pages while
        // keeping the 16-page block intact.
        global.insert_chunk(aligned_base, LARGE_ORDER);
        let isolated_base = aligned_base + LARGE_PAGES * mm::PAGE_SIZE;
        for index in 0..LARGE_PAGES {
            global.insert_chunk(isolated_base + 2 * index * mm::PAGE_SIZE, 0);
        }

        let one_page = Layout::from_size_align(mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();
        let large_block =
            Layout::from_size_align(LARGE_PAGES * mm::PAGE_SIZE, mm::PAGE_SIZE).unwrap();

        assert!(global.alloc_from_pools(&mut local, one_page, 2).is_some());
        assert_eq!(local.total_size(), 7 * mm::PAGE_SIZE);
        assert_eq!(
            global.alloc_from_pools(&mut local, large_block, 2),
            Some(aligned_base)
        );
    }

    #[ktest]
    fn import_rechecks_stale_global_size_snapshot() {
        const NR_CPUS: usize = 4;
        const STALE_SNAPSHOT_BYTES: usize = 17 * 1024 * 1024;
        const ACTUAL_GLOBAL_PAGES: usize = 256;

        let region = MockMemoryRegion::alloc(2 * ACTUAL_GLOBAL_PAGES * mm::PAGE_SIZE);
        let base = region.paddr();
        let _irq = irq::disable_local();
        let mut local = TestBuddySet::<{ pools::MAX_LOCAL_BUDDY_ORDER }>::new_empty(&region);
        let mut global = pools::test::TestGlobalPool::new(&region);

        for index in 0..ACTUAL_GLOBAL_PAGES {
            global.insert_chunk(base + 2 * index * mm::PAGE_SIZE, 0);
        }

        // A stale 17 MiB unlocked snapshot predicts a 2 MiB (`512`-page) import
        // budget on 4 CPUs, which would drain all 256 remaining global pages if
        // `balance_with_snapshot` did not recompute against the locked 1 MiB
        // pool (`expected_local_size` = 128 KiB = 32 pages).
        assert!(global.balance_with_snapshot(&mut local, STALE_SNAPSHOT_BYTES, NR_CPUS));
        assert_eq!(local.total_size(), 32 * mm::PAGE_SIZE);
        assert_eq!(
            global.total_size(),
            (ACTUAL_GLOBAL_PAGES - 32) * mm::PAGE_SIZE
        );
    }
}
