// SPDX-License-Identifier: MPL-2.0

//! Support for **unaccepted memory** in Intel TDX guest VMs.
//!
//! # High-level Design
//!
//! This module implements the TDX guest memory-accept flow defined by the
//! Intel TDX module ABI and follows the Linux-compatible EFI unaccepted-memory
//! bitmap format used at boot.
//!
//! The implementation is built around three cooperating components:
//!
//! ## 1. Firmware-provided bitmap (correctness source)
//!
//! The EFI stub publishes a Linux-compatible *unaccepted-memory bitmap* via the
//! EFI configuration table.  Each bit represents whether a physical memory unit
//! is still pending acceptance.  This bitmap is the **single authoritative
//! correctness source** for unaccepted memory.
//!
//! ## 2. Deferred reservoir (storage-level tracking)
//!
//! Usable but unaccepted memory discovered during early boot is *deferred* into
//! a bounded in-kernel reservoir.  The reservoir stores coarse-grained physical
//! ranges so that acceptance work can be amortized and performed later.
//!
//! The reservoir is segmented by physical address to bound fragmentation and
//! lock contention.  Its capacity is intentionally finite; when the reservoir
//! is full, memory may be returned directly to the global allocator, relying on
//! on-demand acceptance as a safe fallback.
//!
//! ## 3. Shard-based accept protocol (execution-level concurrency)
//!
//! Acceptance is performed by `accept_with_shard_locks`, which uses shard-level
//! locking and *in-flight* tracking to coordinate concurrent accept operations:
//!
//!  * bitmap bits are claimed under a shard lock,
//!  * slow platform accepts run without holding locks,
//!  * in-flight ranges distinguish "accepted" from "claimed but still in progress".
//!
//! This ensures that no caller ever observes memory as usable before acceptance
//! has truly completed, even under heavy concurrency.
//!
//! # Allocation-time Safety Guarantee
//!
//! The global frame allocator is **not assumed** to contain only accepted memory
//! (e.g. when the deferred reservoir was full during initialization).  Therefore,
//! every physical allocation path enforces the following invariant:
//!
//! > **Memory is accepted before it is first accessed (including zeroing).**
//!
//! This invariant makes the design robust against false negatives from lock-free
//! bitmap reads and guarantees correctness even when deferral capacity is
//! exceeded.
//!
//! # Core Data Structures
//!
//! The implementation relies on a small number of core data structures with
//! clearly separated responsibilities:
//!
//! * **`EfiUnacceptedMemory` (bitmap table)**
//!   Firmware-provided bitmap tracking which physical memory units are still
//!   pending acceptance.  This is the authoritative correctness source.
//!
//! * **Bitmap shards (`ShardState`)**
//!   Per-shard concurrency control for acceptance.  Shards protect bitmap
//!   updates and track *in-flight* accept operations so that memory is never
//!   observed as usable before acceptance completes.
//!
//! * **Deferred reservoir segments (`DeferredSegmentState`)**
//!   Bounded storage for usable-but-unaccepted memory ranges discovered during
//!   early boot.  Segments group deferred ranges by physical address to bound
//!   fragmentation and lock contention.

use alloc::sync::Arc;
use core::{
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use spin::Once;
use tdx_guest::{AcceptError, accept_memory, unaccepted_memory::EfiUnacceptedMemory};

use crate::{
    mm::{PAGE_SIZE, Paddr},
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard},
};

/// Sets the unaccepted-memory table pointer parsed at boot entry.
pub(crate) fn set_unaccepted_memory_table(table: Option<&'static mut EfiUnacceptedMemory>) {
    match table {
        Some(t) => {
            crate::info!("Set unaccepted memory table pointer to {:p}", t);
            REFILL_GRANULARITY_BYTES.store(
                usize::try_from(u64::from(t.unit_size_bytes()))
                    .ok()
                    .map(|bytes| bytes.max(PAGE_SIZE).align_up(PAGE_SIZE))
                    .unwrap_or(PAGE_SIZE),
                Ordering::Release,
            );

            let unit_size = u64::from(t.unit_size_bytes());
            // SAFETY: `t` points to valid table memory from boot info.
            let total_bits = unsafe { t.pending_unit_count() }.unwrap_or(0);
            let initial_bytes = total_bits.saturating_mul(unit_size);
            TOTAL_UNACCEPTED_BYTES.store(initial_bytes as usize, Ordering::Release);

            UNACCEPTED_TABLE.store(t as *mut _, Ordering::Release);
        }
        None => {
            REFILL_GRANULARITY_BYTES.store(0, Ordering::Release);
            crate::warn!(
                "Unaccepted memory table is unavailable, lazy-accept bitmap path will be disabled"
            );
        }
    }
}

/// Rewrites the EFI-table pointer to the kernel linear mapping after the kernel page table
/// has been activated.
///
/// # Precondition
///
/// Must be called **before SMP initialization** while only the bootstrap
/// processor (BSP) is running.  No concurrent readers of `UNACCEPTED_TABLE`
/// may exist, because the old physical-address mapping may become invalid
/// after this call.
pub(crate) fn remap_table_ptr_after_paging() {
    let Some(old_ptr) = load_unaccepted_table() else {
        return;
    };

    let old_addr = old_ptr.as_ptr().addr();
    if old_addr < crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR {
        let new_ptr = crate::mm::kspace::paddr_to_vaddr(old_addr) as *mut EfiUnacceptedMemory;
        UNACCEPTED_TABLE.store(new_ptr, Ordering::Release);
        crate::info!(
            "Remapped unaccepted memory table pointer: {:#x} -> {:#x}",
            old_addr,
            new_ptr.addr()
        );
    }
}

/// Initializes unaccepted memory support from the boot-time parsed table pointer.
pub(super) fn init() {
    init_accept_mode_from_cmdline();

    if load_unaccepted_table().is_some() {
        try_eager_accept_memory();
    } else {
        crate::warn!("Unaccepted memory table is unavailable; fallback accept path will be used");
    }
}

/// Attempts to defer a usable range that overlaps unaccepted memory.
pub(super) fn try_defer_usable_memory(addr: Paddr, size: usize) -> DeferOutcome {
    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) || !is_range_unaccepted(addr, size) {
        return DeferOutcome::NotUnaccepted;
    }

    let start = addr.align_up(PAGE_SIZE);
    let end = addr.saturating_add(size).align_down(PAGE_SIZE);
    if start >= end {
        // No page-aligned bytes remain after alignment; nothing to defer.
        return DeferOutcome::NotUnaccepted;
    }

    let mut defer_plan = build_defer_plan(start, end);

    // Sort touched segments by index to enforce a global lock ordering and
    // prevent ABBA deadlocks when concurrent callers touch overlapping segments.
    defer_plan.touched_segments[..defer_plan.num_touched_segments].sort_unstable();

    let mut segment_guards: [Option<SpinLockGuard<'static, DeferredSegmentState, LocalIrqDisabled>>;
        SEGMENT_LOCK_COUNT] = core::array::from_fn(|_| None);

    for plan_index in 0..defer_plan.num_touched_segments {
        let segment_index = defer_plan.touched_segments[plan_index];
        segment_guards[segment_index] = Some(SEGMENT_STATES[segment_index].lock());
    }

    for plan_index in 0..defer_plan.num_touched_segments {
        let segment_index = defer_plan.touched_segments[plan_index];
        let required = defer_plan.required_range_slots[segment_index];
        let segment_state = segment_guards[segment_index].as_ref().unwrap();
        // Account for the fact that push_range may merge with existing
        // adjacent ranges (costing 0 new slots) rather than always appending.
        // Only report full when there is no merge headroom at all.
        let available_slots = segment_state
            .ranges
            .len()
            .saturating_sub(segment_state.num_ranges);
        if available_slots < required && !segment_state.has_merge_potential() {
            return DeferOutcome::ReservoirFull;
        }
    }

    let mut cursor = start;
    let mut deferred_bytes = 0usize;
    while cursor < end {
        let segment_index = segment_index_of(cursor);
        let segment_end = segment_boundary_end(cursor).min(end);
        let chunk_size = segment_end - cursor;

        let segment_state = segment_guards[segment_index].as_mut().unwrap();
        if segment_state.push_range(cursor, chunk_size).is_err() {
            // Optimistic precheck was wrong; compact first then retry once.
            segment_state.compact();
            if segment_state.push_range(cursor, chunk_size).is_err() {
                return DeferOutcome::ReservoirFull;
            }
        }
        deferred_bytes = deferred_bytes.saturating_add(chunk_size);
        cursor = segment_end;
    }

    for plan_index in 0..defer_plan.num_touched_segments {
        mark_segment_nonempty(defer_plan.touched_segments[plan_index]);
    }

    TOTAL_DEFERRED_BYTES.fetch_add(deferred_bytes, Ordering::Relaxed);
    DeferOutcome::Deferred
}

/// Tries to refill accepted memory from the deferred unaccepted reservoir.
pub(super) fn try_refill_for_allocation(layout: core::alloc::Layout) -> Result<bool, AcceptError> {
    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return Ok(false);
    }

    let request_bytes = layout
        .size()
        .max(layout.align())
        .align_up(PAGE_SIZE)
        .max(PAGE_SIZE);
    let target_chunk_bytes = request_bytes.max(get_refill_granularity_bytes());
    let refill_budget_bytes = calculate_refill_budget_bytes(target_chunk_bytes);

    let accepted_bytes = refill_deferred_memory(target_chunk_bytes, refill_budget_bytes)?;
    Ok(accepted_bytes > 0)
}

/// Accepts memory on demand if the target range is still unaccepted.
pub(super) fn accept_memory_if_needed(addr: Paddr, size: usize) -> Result<(), AcceptError> {
    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return Ok(());
    }

    let Some(table_ptr) = load_unaccepted_table() else {
        return Ok(());
    };

    let end = addr
        .checked_add(size)
        .ok_or(AcceptError::ArithmeticOverflow)?;

    // SAFETY: `table_ptr` is initialized from boot info and points to valid table memory.
    let table = unsafe { &*table_ptr.as_ptr() };

    // Go through shard locks with in-flight tracking: accept_with_shard_locks
    // will spin-wait if the target range overlaps a concurrent in-progress
    // accept, ensuring we only return Ok(()) once the memory is truly accepted.
    accept_with_shard_locks(table, addr as u64, end as u64)
}

/// Returns an advisory hint on whether the physical range `[addr, addr + size)`
/// may still overlap unaccepted memory.
pub(super) fn is_range_unaccepted(addr: Paddr, size: usize) -> bool {
    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return false;
    }

    let Some(table_ptr) = load_unaccepted_table() else {
        return false;
    };

    // SAFETY: `table_ptr` is initialized from boot info and points to valid table memory.
    let table = unsafe { &*table_ptr.as_ptr() };
    let Some(end) = addr.checked_add(size) else {
        return true;
    };

    // Lockless advisory bitmap read only.
    // Concurrent accept operations can make the bitmap temporarily disagree with
    // the true usability state of the range, so this check must not be used as a
    // correctness or safety decision. At most it is a policy hint for deferral /
    // refill decisions; allocation-time safety is enforced by
    // `accept_memory_if_needed`.
    //
    // SAFETY: `table_ptr` is initialized from boot info, the bitmap memory
    // is valid, and `is_range_pending` uses atomic reads so it is safe under
    // concurrent bitmap mutation.
    unsafe { table.is_range_pending(addr as u64, end as u64) }.unwrap_or(true)
}

pub(super) fn load_total_unaccepted_bytes() -> usize {
    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return 0;
    }
    TOTAL_UNACCEPTED_BYTES.load(Ordering::Relaxed)
}

/// Spawns background worker threads that gradually accept deferred memory.
pub(super) fn spawn_background_accept_worker(
    spawner: impl Fn(alloc::boxed::Box<dyn FnOnce() + Send>),
    sleeper: impl Fn(core::time::Duration) + Send + Sync + 'static,
) {
    if get_accept_memory_mode() != AcceptMemoryMode::LazyBackground
        || BACKGROUND_WORKER_STARTED.swap(true, Ordering::AcqRel)
    {
        return;
    }

    let worker_count = background_worker_count();
    let sleeper = Arc::new(sleeper);

    for worker_id in 0..worker_count {
        let sleeper = Arc::clone(&sleeper);
        spawner(alloc::boxed::Box::new(move || {
            background_accept_worker_loop(worker_id, sleeper)
        }));
    }

    crate::info!(
        "lazy-background accept workers started: count={}",
        worker_count
    );
}

/// Outcome of attempting to defer a usable memory range to the unaccepted reservoir.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DeferOutcome {
    /// The range does not contain unaccepted memory; the caller should add it
    /// to the frame allocator directly.
    NotUnaccepted,
    /// The range was deferred to the reservoir for later acceptance.
    Deferred,
    /// The reservoir is full; the caller should add the range to the frame
    /// allocator directly (on-demand acceptance will handle it).
    ReservoirFull,
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum AcceptMemoryMode {
    #[default]
    Lazy,
    LazyBackground,
    Eager,
}

impl core::str::FromStr for AcceptMemoryMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // The cmdline framework normalizes hyphens to underscores, but OSTD
        // parses before the framework runs, so accept both forms.
        match s {
            "lazy" => Ok(AcceptMemoryMode::Lazy),
            "lazy-background" | "lazy_background" => Ok(AcceptMemoryMode::LazyBackground),
            "eager" => Ok(AcceptMemoryMode::Eager),
            _ => Err(()),
        }
    }
}

fn init_accept_mode_from_cmdline() {
    let mode = parse_accept_mode_from_cmdline();
    ACCEPT_MEMORY_MODE.call_once(|| mode);

    match mode {
        AcceptMemoryMode::Lazy => crate::info!("accept_memory mode: lazy"),
        AcceptMemoryMode::LazyBackground => crate::info!("accept_memory mode: lazy-background"),
        AcceptMemoryMode::Eager => crate::info!("accept_memory mode: eager"),
    }
}

fn parse_accept_mode_from_cmdline() -> AcceptMemoryMode {
    let Some(boot_info) = crate::boot::EARLY_INFO.get() else {
        return AcceptMemoryMode::default();
    };

    let value = boot_info
        .kernel_cmdline
        .split_whitespace()
        .find(|arg| arg.starts_with("accept_memory="))
        .and_then(|arg| arg.split_once('='))
        .map(|(_, value)| value);

    match value {
        Some(v) => v.parse().unwrap_or_else(|()| {
            crate::warn!("unknown accept_memory mode '{}', fallback to lazy", v);
            AcceptMemoryMode::default()
        }),
        None => AcceptMemoryMode::default(),
    }
}

fn try_eager_accept_memory() {
    if get_accept_memory_mode() != AcceptMemoryMode::Eager {
        return;
    }

    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return;
    }

    let Some(table_ptr) = load_unaccepted_table() else {
        crate::warn!("accept_memory=eager requested but unaccepted table is unavailable");
        return;
    };

    // SAFETY: `table_ptr` is initialized from boot info and points to valid table memory.
    let table = unsafe { &*table_ptr.as_ptr() };

    let table_phys_base = table.phys_base();

    let Some(coverage_end) = table.bitmap_coverage_end() else {
        let table_bitmap_size_bytes = table.bitmap_size_bytes();
        let table_unit_size_bytes = table.unit_size_bytes();

        crate::error!(
            "unaccepted bitmap coverage overflow: bitmap_size_bytes={}, unit_size_bytes={}, phys_base={:#x}",
            table_bitmap_size_bytes,
            table_unit_size_bytes,
            table_phys_base
        );
        return;
    };

    crate::early_println!("[kernel] Accepting all unaccepted memory ...");

    if accept_with_shard_locks(table, table_phys_base, coverage_end).is_ok() {
        // Clear deferred ranges first so that observers who see
        // `EAGER_ACCEPT_COMPLETED == true` also see an empty reservoir,
        // maintaining the invariant: completed -> reservoir empty.
        clear_deferred_ranges();
        EAGER_ACCEPT_COMPLETED.store(true, Ordering::Release);
        crate::info!(
            "accept_memory=eager completed: accepted bitmap coverage [{:#x}, {:#x})",
            table_phys_base,
            coverage_end
        );
    } else {
        crate::error!(
            "accept_memory=eager failed: range=[{:#x}, {:#x})",
            table_phys_base,
            coverage_end
        );
    }
}

/// Accepts a GPA range using shard-level locking for multi-CPU parallelism.
///
/// Uses a **claim → accept → complete** pattern with per-shard in-flight
/// tracking to avoid holding IRQ-disabled locks during the slow TDX TDCALL
/// operations while correctly distinguishing "accepted" from "in-progress":
///
/// 1. **Claim** (shard lock held, IRQs disabled — brief): scan bitmap for the
///    next contiguous run of pending bits, clear them, and register the run in
///    the shard's in-flight list.
/// 2. **Accept** (no lock, IRQs enabled — slow): perform TDX acceptance.
/// 3. **Complete** (shard lock held — brief): remove from in-flight list.
///    On error, additionally restore the bitmap bits (rollback).
///
/// Concurrent callers that see bitmap bits already cleared will check the
/// in-flight list: if the range overlaps an in-flight entry, they spin-wait
/// until the accepting CPU completes, ensuring no caller returns `Ok(())`
/// for memory that is still mid-accept.
fn accept_with_shard_locks(
    table: &EfiUnacceptedMemory,
    start: u64,
    end: u64,
) -> Result<(), AcceptError> {
    if start >= end {
        return Ok(());
    }

    let mut cursor = start;
    let mut retry_count = 0u32;
    while cursor < end {
        let shard_index = bitmap_shard_index(cursor);
        let shard_end = bitmap_shard_boundary_end(cursor).min(end);

        // Phase 1: Under shard lock — claim one pending run and register it
        // as in-flight so concurrent callers can distinguish "accepted" from
        // "in-progress".
        let mut retry = false;
        let claimed = {
            let mut shard = BITMAP_SHARD_LOCKS[shard_index].lock();
            if shard.num_inflight >= MAX_INFLIGHT_PER_SHARD {
                retry = true;
                None
            } else {
                // SAFETY: Shard lock guarantees exclusive access to this bitmap region.
                let run = unsafe { table.claim_next_pending_run(cursor, shard_end)? };
                if let Some((rs, re)) = run {
                    debug_assert!(
                        rs >= cursor && re <= shard_end && rs < re,
                        "claim_next_pending_run returned out-of-bounds run: \
                         cursor={:#x}, shard_end={:#x}, run=[{:#x}, {:#x})",
                        cursor,
                        shard_end,
                        rs,
                        re
                    );
                    if rs > cursor && shard.has_inflight_overlap(cursor, rs) {
                        // SAFETY: Shard lock guarantees exclusive access.
                        let _ = unsafe { table.restore_pending_range(rs, re) };
                        retry = true;
                        None
                    } else {
                        shard.add_inflight(rs, re);
                        Some((rs, re))
                    }
                } else {
                    None
                }
            }
        };

        if retry {
            retry_count += 1;
            for _ in 0..(1u32 << retry_count.min(6)) {
                core::hint::spin_loop();
            }
            continue;
        }
        retry_count = 0;

        let Some((run_start, run_end)) = claimed else {
            let has_overlap = {
                let shard = BITMAP_SHARD_LOCKS[shard_index].lock();
                shard.has_inflight_overlap(cursor, shard_end)
            };
            if has_overlap {
                retry_count += 1;
                for _ in 0..(1u32 << retry_count.min(6)) {
                    core::hint::spin_loop();
                }
                continue;
            }
            // No pending and no in-flight: range is truly accepted.
            cursor = shard_end;
            continue;
        };

        // Phase 2: Perform the slow TDX accept with IRQs enabled.
        // SAFETY: The claimed range is exclusively ours (bits cleared + inflight).
        let accept_result = unsafe { accept_memory(run_start, run_end) };

        // Phase 3: Under shard lock — remove from in-flight, rollback on error.
        {
            let mut shard = BITMAP_SHARD_LOCKS[shard_index].lock();
            shard.remove_inflight(run_start, run_end);
            if accept_result.is_err() {
                // SAFETY: Shard lock guarantees exclusive access.
                let _ = unsafe { table.restore_pending_range(run_start, run_end) };
            }
        }

        match accept_result {
            Ok(()) => {
                let accepted_bytes = (run_end - run_start) as usize;
                TOTAL_UNACCEPTED_BYTES.fetch_sub(accepted_bytes, Ordering::Relaxed);
                cursor = run_end;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

fn refill_deferred_memory(
    target_chunk_bytes: usize,
    refill_budget_bytes: usize,
) -> Result<usize, AcceptError> {
    let mut accepted_total = 0usize;
    let mut remaining_budget_bytes = refill_budget_bytes.max(PAGE_SIZE);
    let base_seed = current_cpu_segment_seed();

    while remaining_budget_bytes >= PAGE_SIZE {
        if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
            return Ok(accepted_total);
        }

        // Vary the seed across iterations so successive reserves within the
        // same refill call spread across different segments.
        let seed = (base_seed + accepted_total / PAGE_SIZE) % SEGMENT_LOCK_COUNT;

        let desired_chunk_bytes = target_chunk_bytes
            .min(remaining_budget_bytes)
            .max(PAGE_SIZE)
            .align_up(PAGE_SIZE);

        let Some((segment_index, chunk_addr, chunk_len)) =
            reserve_deferred_chunk(seed, desired_chunk_bytes)
        else {
            return Ok(accepted_total);
        };

        match accept_chunk_via_bitmap_table(chunk_addr, chunk_len) {
            Ok(()) => {
                TOTAL_DEFERRED_BYTES.fetch_sub(chunk_len, Ordering::Relaxed);
                super::allocator::get_global_frame_allocator()
                    .add_free_memory(chunk_addr, chunk_len);
                accepted_total = accepted_total.saturating_add(chunk_len);
                remaining_budget_bytes = remaining_budget_bytes.saturating_sub(chunk_len);
            }
            Err(err) => {
                return_deferred_chunk(segment_index, chunk_addr, chunk_len);
                return Err(err);
            }
        }
    }

    Ok(accepted_total)
}

fn build_defer_plan(start: Paddr, end: Paddr) -> DeferPlan {
    let mut required_range_slots = [0; SEGMENT_LOCK_COUNT];
    let mut touched_segments = [0; SEGMENT_LOCK_COUNT];
    let mut num_touched_segments = 0;
    let mut cursor = start;

    while cursor < end {
        let segment_index = segment_index_of(cursor);
        if required_range_slots[segment_index] == 0 {
            touched_segments[num_touched_segments] = segment_index;
            num_touched_segments += 1;
        }
        required_range_slots[segment_index] += 1;
        cursor = segment_boundary_end(cursor).min(end);
    }

    DeferPlan {
        required_range_slots,
        touched_segments,
        num_touched_segments,
    }
}

fn accept_chunk_via_bitmap_table(addr: Paddr, len: usize) -> Result<(), AcceptError> {
    let Some(table_ptr) = load_unaccepted_table() else {
        return Ok(());
    };

    let end = addr
        .checked_add(len)
        .ok_or(AcceptError::ArithmeticOverflow)?;

    // SAFETY: `table_ptr` is initialized from boot info and points to valid table memory.
    let table = unsafe { &*table_ptr.as_ptr() };

    accept_with_shard_locks(table, addr as u64, end as u64)
}

fn reserve_deferred_chunk(seed: usize, desired_chunk: usize) -> Option<(usize, Paddr, usize)> {
    let nonempty_hint = NONEMPTY_SEGMENT_HINT_BITMAP.load(Ordering::Acquire);
    if nonempty_hint == 0 {
        return None;
    }

    for offset in 0..SEGMENT_LOCK_COUNT {
        let segment_index = (seed + offset) % SEGMENT_LOCK_COUNT;
        if !is_segment_marked_nonempty(nonempty_hint, segment_index) {
            continue;
        }

        let mut segment_state = SEGMENT_STATES[segment_index].lock();
        if let Some((addr, len)) = segment_state.pop_chunk(desired_chunk) {
            if segment_state.num_ranges == 0 {
                mark_segment_empty(segment_index);
            }
            return Some((segment_index, addr, len));
        }

        mark_segment_empty(segment_index);
    }

    None
}

fn return_deferred_chunk(segment_index: usize, addr: Paddr, size: usize) {
    let mut segment_state = SEGMENT_STATES[segment_index].lock();
    if segment_state.push_range(addr, size).is_err() {
        segment_state.compact();
        if segment_state.push_range(addr, size).is_err() {
            // Reservoir full even after compaction — drop the chunk.
            // The memory remains pending in the bitmap (bits were restored
            // by accept_with_shard_locks), so on-demand accept will handle
            // it.  Adjust TOTAL_DEFERRED_BYTES to keep the counter accurate.
            TOTAL_DEFERRED_BYTES.fetch_sub(size, Ordering::Relaxed);
            crate::warn!(
                "reservoir rollback overflow: segment={}, addr={:#x}, size={:#x}; \
                 chunk dropped from reservoir",
                segment_index,
                addr,
                size
            );
            return;
        }
    }
    mark_segment_nonempty(segment_index);
}

fn clear_deferred_ranges() {
    for segment in SEGMENT_STATES.iter().take(SEGMENT_LOCK_COUNT) {
        let mut segment_state = segment.lock();
        segment_state.clear();
    }
    NONEMPTY_SEGMENT_HINT_BITMAP.store(0, Ordering::Release);
    TOTAL_DEFERRED_BYTES.store(0, Ordering::Relaxed);
}

fn calculate_refill_budget_bytes(target_chunk_bytes: usize) -> usize {
    let extra_budget_bytes =
        target_chunk_bytes.clamp(MIN_REFILL_EXTRA_BUDGET_BYTES, MAX_REFILL_EXTRA_BUDGET_BYTES);
    target_chunk_bytes.saturating_add(extra_budget_bytes)
}

fn get_refill_granularity_bytes() -> usize {
    let cached = REFILL_GRANULARITY_BYTES.load(Ordering::Acquire);
    if cached != 0 {
        return cached;
    }

    let Some(table_ptr) = load_unaccepted_table() else {
        return PAGE_SIZE;
    };

    // SAFETY: `table_ptr` is initialized from boot info and points to valid table memory.
    let table = unsafe { &*table_ptr.as_ptr() };
    let granularity = usize::try_from(u64::from(table.unit_size_bytes()))
        .ok()
        .map(|bytes| bytes.max(PAGE_SIZE).align_up(PAGE_SIZE))
        .unwrap_or(PAGE_SIZE);
    REFILL_GRANULARITY_BYTES.store(granularity, Ordering::Release);
    granularity
}

/// Returns a per-CPU seed for distributing segment accesses.
///
/// Uses the current CPU ID (racy but sufficient for load distribution) to avoid
/// contention on a single global atomic counter under high-concurrency refill.
fn current_cpu_segment_seed() -> usize {
    u32::from(crate::cpu::CpuId::current_racy()) as usize
}

fn is_reservoir_empty() -> bool {
    NONEMPTY_SEGMENT_HINT_BITMAP.load(Ordering::Acquire) == 0
        && TOTAL_DEFERRED_BYTES.load(Ordering::Relaxed) == 0
}

fn load_unaccepted_table() -> Option<NonNull<EfiUnacceptedMemory>> {
    NonNull::new(UNACCEPTED_TABLE.load(Ordering::Acquire))
}

fn get_accept_memory_mode() -> AcceptMemoryMode {
    ACCEPT_MEMORY_MODE.get().copied().unwrap_or_default()
}

fn is_segment_marked_nonempty(nonempty_hint: u64, segment_index: usize) -> bool {
    debug_assert!(segment_index < SEGMENT_LOCK_COUNT);
    (nonempty_hint & segment_mask(segment_index)) != 0
}

fn mark_segment_nonempty(segment_index: usize) {
    NONEMPTY_SEGMENT_HINT_BITMAP.fetch_or(segment_mask(segment_index), Ordering::Release);
}

fn mark_segment_empty(segment_index: usize) {
    NONEMPTY_SEGMENT_HINT_BITMAP.fetch_and(!segment_mask(segment_index), Ordering::Release);
}

const fn segment_mask(segment_index: usize) -> u64 {
    1u64 << segment_index
}

fn bitmap_shard_index(addr: u64) -> usize {
    (addr as usize / SEGMENT_BYTES) % SEGMENT_LOCK_COUNT
}

fn bitmap_shard_boundary_end(addr: u64) -> u64 {
    let shard_start = (addr / SEGMENT_BYTES as u64) * SEGMENT_BYTES as u64;
    shard_start.saturating_add(SEGMENT_BYTES as u64)
}

fn segment_index_of(addr: Paddr) -> usize {
    (addr / SEGMENT_BYTES) % SEGMENT_LOCK_COUNT
}

fn segment_boundary_end(addr: Paddr) -> Paddr {
    let segment_start = (addr / SEGMENT_BYTES) * SEGMENT_BYTES;
    segment_start.saturating_add(SEGMENT_BYTES)
}

fn background_accept_worker_loop(
    worker_id: usize,
    sleeper: Arc<dyn Fn(core::time::Duration) + Send + Sync>,
) {
    use core::{
        sync::atomic::Ordering::{Acquire, Release},
        time::Duration,
    };

    const ITERATION_SLEEP_MS: u64 = 20;
    const IDLE_SLEEP_MS: u64 = 200;

    crate::info!("lazy-background accept worker {} started", worker_id);

    if load_unaccepted_table().is_none() {
        return;
    }

    let sleep_fn = |ms| sleeper(Duration::from_millis(ms));

    while !BACKGROUND_WORKER_DISABLED.load(Acquire) {
        if EAGER_ACCEPT_COMPLETED.load(Acquire) || BACKGROUND_WORKER_DISABLED.load(Acquire) {
            break;
        }

        match refill_deferred_memory(
            BACKGROUND_REFILL_CHUNK_BYTES,
            BACKGROUND_REFILL_BUDGET_BYTES,
        ) {
            Ok(0) => {
                if is_reservoir_empty() {
                    break;
                }
                sleep_fn(IDLE_SLEEP_MS);
            }
            Ok(_) => sleep_fn(ITERATION_SLEEP_MS),
            Err(err) => {
                crate::error!("Background accept worker {} failed: {:?}", worker_id, err);
                BACKGROUND_WORKER_DISABLED.store(true, Release);
                break;
            }
        }
    }

    if is_reservoir_empty() && !BACKGROUND_WORKER_DISABLED.load(Acquire) {
        crate::info!(
            "lazy-background accept worker {} drained deferred reservoir",
            worker_id
        );
    } else if BACKGROUND_WORKER_DISABLED.load(Acquire) {
        crate::warn!(
            "lazy-background accept worker {} stopped: background refill disabled",
            worker_id
        );
    }
}

fn background_worker_count() -> usize {
    // Use a small fraction of CPUs for background acceptance to avoid
    // starving foreground workloads with lock contention and TDCALL overhead.
    let cpus = crate::cpu::num_cpus();
    (cpus / 4).clamp(1, BACKGROUND_WORKER_MAX)
}

struct DeferPlan {
    required_range_slots: [usize; SEGMENT_LOCK_COUNT],
    touched_segments: [usize; SEGMENT_LOCK_COUNT],
    num_touched_segments: usize,
}

struct DeferredSegmentState {
    ranges: [DeferredRange; MAX_DEFERRED_RANGES],
    num_ranges: usize,
    /// Round-robin cursor for `pop_chunk` to avoid always scanning from index 0.
    pop_cursor: usize,
}

impl DeferredSegmentState {
    const fn new() -> Self {
        Self {
            ranges: [DeferredRange::EMPTY; MAX_DEFERRED_RANGES],
            num_ranges: 0,
            pop_cursor: 0,
        }
    }

    fn has_merge_potential(&self) -> bool {
        self.num_ranges >= 1
    }

    fn push_range(&mut self, addr: Paddr, size: usize) -> Result<(), ReservoirFull> {
        let start = addr.align_up(PAGE_SIZE);
        let end = addr.saturating_add(size).align_down(PAGE_SIZE);
        if start >= end {
            return Ok(());
        }

        // Try to merge with an existing adjacent range before appending.
        for index in 0..self.num_ranges {
            let r = &mut self.ranges[index];
            if r.end == start {
                r.end = end;
                return Ok(());
            }
            if end == r.start {
                r.start = start;
                return Ok(());
            }
        }

        // Compact when num_ranges exceeds 3/4 capacity to curb fragmentation.
        if self.num_ranges >= self.ranges.len() * 3 / 4 {
            self.compact();
        }

        if self.num_ranges >= self.ranges.len() {
            return Err(ReservoirFull);
        }

        self.ranges[self.num_ranges] = DeferredRange { start, end };
        self.num_ranges += 1;
        Ok(())
    }

    /// Sorts ranges by start address and merges adjacent/overlapping entries.
    fn compact(&mut self) {
        if self.num_ranges <= 1 {
            return;
        }

        // Introsort: O(n log n) worst case, no allocation, available in core.
        self.ranges[..self.num_ranges].sort_unstable_by_key(|r| r.start);

        // Merge adjacent/overlapping ranges in place.
        let mut write = 0;
        for read in 1..self.num_ranges {
            if self.ranges[read].start <= self.ranges[write].end {
                // Overlapping or adjacent — extend the current range.
                if self.ranges[read].end > self.ranges[write].end {
                    self.ranges[write].end = self.ranges[read].end;
                }
            } else {
                write += 1;
                self.ranges[write] = self.ranges[read];
            }
        }
        let new_len = write + 1;

        // Clear stale tail entries.
        for i in new_len..self.num_ranges {
            self.ranges[i] = DeferredRange::EMPTY;
        }
        self.num_ranges = new_len;
    }

    fn pop_chunk(&mut self, target_len: usize) -> Option<(Paddr, usize)> {
        let aligned_target = target_len.max(PAGE_SIZE).align_up(PAGE_SIZE);
        let start_cursor = self.pop_cursor.min(self.num_ranges.saturating_sub(1));
        for offset in 0..self.num_ranges {
            let index = (start_cursor + offset) % self.num_ranges;
            let range = self.ranges[index];
            if range.is_empty() {
                continue;
            }

            let chunk_len = range.len().min(aligned_target).align_down(PAGE_SIZE);
            if chunk_len >= PAGE_SIZE {
                let start = range.start;
                self.ranges[index].start = self.ranges[index].start.saturating_add(chunk_len);
                if self.ranges[index].is_empty() {
                    self.remove_range(index);
                }
                self.pop_cursor = index.wrapping_add(1);
                return Some((start, chunk_len));
            }
        }

        None
    }

    fn remove_range(&mut self, index: usize) {
        debug_assert!(index < self.num_ranges);
        self.num_ranges -= 1;
        self.ranges[index] = self.ranges[self.num_ranges];
        self.ranges[self.num_ranges] = DeferredRange::EMPTY;
    }

    fn clear(&mut self) {
        self.num_ranges = 0;
        self.pop_cursor = 0;
        self.ranges.fill(DeferredRange::EMPTY);
    }
}

#[derive(Clone, Copy)]
struct DeferredRange {
    start: Paddr,
    end: Paddr,
}

impl DeferredRange {
    const EMPTY: Self = Self { start: 0, end: 0 };

    const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    const fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// Per-shard state tracking ranges currently being accepted (in-flight).
///
/// Protected by the corresponding shard's [`SpinLock`].  This allows
/// concurrent callers to distinguish bitmap bit 0 meaning "accepted"
/// from "claimed/in-progress", preventing premature use of unaccepted memory.
struct ShardState {
    inflight: [(u64, u64); MAX_INFLIGHT_PER_SHARD],
    num_inflight: usize,
}

impl ShardState {
    const fn new() -> Self {
        Self {
            inflight: [(0, 0); MAX_INFLIGHT_PER_SHARD],
            num_inflight: 0,
        }
    }

    fn add_inflight(&mut self, start: u64, end: u64) {
        debug_assert!(self.num_inflight < MAX_INFLIGHT_PER_SHARD);
        self.inflight[self.num_inflight] = (start, end);
        self.num_inflight += 1;
    }

    fn remove_inflight(&mut self, start: u64, end: u64) {
        for i in 0..self.num_inflight {
            if self.inflight[i] == (start, end) {
                self.num_inflight -= 1;
                self.inflight[i] = self.inflight[self.num_inflight];
                self.inflight[self.num_inflight] = (0, 0);
                return;
            }
        }
        debug_assert!(
            false,
            "remove_inflight: entry ({:#x}, {:#x}) not found",
            start, end
        );
    }

    fn has_inflight_overlap(&self, start: u64, end: u64) -> bool {
        for i in 0..self.num_inflight {
            let (rs, re) = self.inflight[i];
            if rs < end && re > start {
                return true;
            }
        }
        false
    }
}

/// Error returned when a segment's deferred-range array is full.
#[derive(Debug)]
struct ReservoirFull;

// Maximum number of deferred (unaccepted) memory ranges tracked per deferred segment.
// This bounds fragmentation tolerance.
// In typical EFI firmware setups, the actual number of ranges
// is expected to be very small (often < 10). Larger values only matter
// if firmware reports heavily fragmented unaccepted memory.
const MAX_DEFERRED_RANGES: usize = 512;

// Maximum number of concurrent in-flight accept operations per bitmap shard.
const MAX_INFLIGHT_PER_SHARD: usize = 8;

// Number of deferred-memory segments and the matching lock / hint-bitmap slots.
const SEGMENT_LOCK_COUNT: usize = 64;
// Physical address span covered by one deferred segment and one bitmap shard.
const SEGMENT_BYTES: usize = 256 * 1024 * 1024;

const MIN_REFILL_EXTRA_BUDGET_BYTES: usize = 16 * 1024 * 1024;
const MAX_REFILL_EXTRA_BUDGET_BYTES: usize = 64 * 1024 * 1024;

// Target chunk size a background worker tries to accept per successful refill.
const BACKGROUND_REFILL_CHUNK_BYTES: usize = 2 * 1024 * 1024;
// Maximum acceptance work a background worker performs in one refill iteration.
const BACKGROUND_REFILL_BUDGET_BYTES: usize = 4 * 1024 * 1024;
const BACKGROUND_WORKER_MAX: usize = 16;

static UNACCEPTED_TABLE: AtomicPtr<EfiUnacceptedMemory> = AtomicPtr::new(core::ptr::null_mut());
static ACCEPT_MEMORY_MODE: Once<AcceptMemoryMode> = Once::new();
static EAGER_ACCEPT_COMPLETED: AtomicBool = AtomicBool::new(false);

static TOTAL_UNACCEPTED_BYTES: AtomicUsize = AtomicUsize::new(0);
static REFILL_GRANULARITY_BYTES: AtomicUsize = AtomicUsize::new(0);
/// Tracks the number of bytes currently stored in the deferred reservoir.
static TOTAL_DEFERRED_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Per-shard state that protects the unaccepted bitmap region and tracks
/// in-flight accept operations.  Each shard covers `SEGMENT_BYTES` of
/// physical address space, matching the deferred-range segment granularity.
static BITMAP_SHARD_LOCKS: [SpinLock<ShardState, LocalIrqDisabled>; SEGMENT_LOCK_COUNT] =
    [const { SpinLock::new(ShardState::new()) }; SEGMENT_LOCK_COUNT];
static SEGMENT_STATES: [SpinLock<DeferredSegmentState, LocalIrqDisabled>; SEGMENT_LOCK_COUNT] =
    [const { SpinLock::new(DeferredSegmentState::new()) }; SEGMENT_LOCK_COUNT];
/// Bit-per-segment hint bitmap used to skip obviously empty deferred segments.
static NONEMPTY_SEGMENT_HINT_BITMAP: AtomicU64 = AtomicU64::new(0);

static BACKGROUND_WORKER_STARTED: AtomicBool = AtomicBool::new(false);
static BACKGROUND_WORKER_DISABLED: AtomicBool = AtomicBool::new(false);
