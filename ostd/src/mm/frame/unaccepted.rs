// SPDX-License-Identifier: MPL-2.0

//! Unaccepted-memory management for Intel TDX guests.
//!
//! # Background
//!
//! Under Intel Trust Domain Extensions (TDX), physical memory assigned to the guest is initially
//! marked as *unaccepted*. Accessing unaccepted physical frames triggers a hardware virtualization
//! exception (`#VE`). To make physical memory usable, the kernel must explicitly accept it via the
//! `TDG.MEM.PAGE.ACCEPT` TDCALL, which can operate at 2 MiB (huge page) or 4 KiB granularity.
//!
//! # Architecture & Strategies
//!
//! This module supports two memory acceptance modes:
//!
//! 1. **Eager Mode**: Memory is partitioned evenly across all available CPUs during early boot,
//!    allowing APs to accept the entire physical address space in parallel before userspace starts.
//! 2. **Lazy Mode (Default)**: Accepting gigabytes of memory eagerly incurs severe boot latency.
//!    Instead, only critical bootstrap regions and sub-unit edge frames are accepted early. The bulk
//!    of usable physical memory is deferred to a designated reservoir and accepted on demand.
//!
//! # Concurrency & Invariants (Lazy Mode)
//!
//! Lazy acceptance uses per-shard fixed-capacity reservoirs and shard-level
//! synchronization:
//!
//! - **Deferred Reservoir (`DEFERRED_SHARDS`)**: Deferred physical ranges are
//!   assigned to one of 64 logical shards using their physical segment index.
//!   Each shard uses a fixed-capacity range array, so reservoir management does
//!   not require dynamic allocation during early boot.
//! - **Two-Phase Boot Ingestion**: BSP initialization first performs a
//!   preflight capacity check for all affected shards, then commits the ranges.
//!   This provides all-or-nothing ingestion when the fixed-capacity reservoir
//!   cannot accommodate the complete input range.
//! - **Atomic Shard Hinting (`NONEMPTY_SHARD_HINT_BITMAP`)**: A 64-bit atomic
//!   bitmap provides a lock-free hint for locating non-empty shards.
//! - **Inflight Acceptance Tracking (`ShardState`)**: Shard locks protect
//!   bitmap claims, while an inflight interval tracker records ranges currently
//!   undergoing acceptance. This prevents overlapping or duplicate acceptance
//!   attempts while allowing the lock to be released during the long-latency
//!   TDCALL.
//! - **RAII Reservations (`DeferredChunkReservation`)**: Chunks removed from
//!   the reservoir are held by a reservation guard. Failed acceptance restores
//!   the chunk to its shard during `Drop`.
//! - **Unit-Aligned Batching**: Deferred ranges and refill chunks are aligned
//!   to the EFI table's `unit_size`, avoiding sub-unit fragmentation and
//!   matching the granularity used by the acceptance bitmap.

use core::{
    alloc::Layout,
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use linux_boot_params::{BootParams, EfiInfo};
use spin::Once;
use tdx_guest::{AcceptError, accept_memory, unaccepted_memory::EfiUnacceptedMemory};

use crate::{
    boot::memory_region::{MemoryRegion, MemoryRegionType},
    cpu::CpuId,
    mm::Paddr,
    sync::{LocalIrqDisabled, SpinLock},
    util::id_set::Id,
};

/// Initializes the unaccepted-memory table from the EFI stub boot parameters.
pub(crate) fn init(boot_params: &BootParams) {
    let efi_info = boot_params.efi_info;
    if efi_info.efi_loader_signature != EfiInfo::ASTERINAS_LOADER_SIGNATURE {
        return;
    }
    let table_addr = efi_info.efi_systab;
    if table_addr == 0 {
        return;
    }

    let table_ptr =
        crate::mm::kspace::paddr_to_vaddr(table_addr as usize) as *mut EfiUnacceptedMemory;

    crate::info!("Found unaccepted memory table at {:p}", table_ptr);

    // SAFETY: The EFI stub initialized the table and its trailing bitmap at
    // this aligned address, and `table_memory_region` reserves the backing
    // allocation for the kernel lifetime before frame allocators start.
    let table = unsafe { &*table_ptr };

    let unit_size = u64::from(table.unit_size_bytes());
    if unit_size == 0 {
        crate::warn!("Ignoring unaccepted memory table with zero unit size");
        return;
    }
    let total_bits = table.pending_unit_count();
    TOTAL_UNACCEPTED_BYTES.store((total_bits * unit_size) as usize, Ordering::Release);

    // SAFETY: The EFI table remains valid for the kernel lifetime.
    UNACCEPTED_TABLE.call_once(|| unsafe { &*table_ptr });
}

/// Returns the table storage region to reserve before initializing frame allocators.
pub(crate) fn table_memory_region() -> Option<MemoryRegion> {
    let table = load_unaccepted_table()?;
    let base = core::ptr::from_ref(table).addr() - crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR;
    let size = size_of::<EfiUnacceptedMemory>() + table.bitmap_size_bytes() as usize;
    Some(MemoryRegion::new(base, size, MemoryRegionType::Reserved))
}

/// Accepts the BSP's slice of unaccepted memory, waits for all APs to finish,
/// and publishes the accepted memory to the global frame allocator.
///
/// This function must be called only once on the BSP in its boot context
/// after booting all APs.
pub(crate) fn accept_memory_on_bsp() {
    if get_accept_memory_mode() != AcceptMemoryMode::Eager {
        return;
    }

    let Some(table) = accept_memory_on_cpu(CpuId::bsp()) else {
        return;
    };
    while FINISHED_CPU_COUNT.load(Ordering::Acquire) != crate::cpu::num_cpus() {
        core::hint::spin_loop();
    }
    TOTAL_UNACCEPTED_BYTES.store(
        (table.pending_unit_count() * u64::from(table.unit_size_bytes())) as usize,
        Ordering::Release,
    );
    publish_accepted_memory(table);
}

/// Accepts the calling AP's disjoint slice of unaccepted memory and marks completion.
///
/// This function must be called only once per AP in its boot context.
pub(crate) fn accept_memory_on_ap() {
    if get_accept_memory_mode() != AcceptMemoryMode::Eager {
        return;
    }

    accept_memory_on_cpu(CpuId::current_racy());
}

/// Initializes the frame allocator's memory managed by the unaccepted-memory table.
pub(super) fn init_allocator_memory(early_allocated_ranges: &[Range<Paddr>; 2]) {
    let Some(table) = load_unaccepted_table() else {
        add_free_ranges(super::allocator::free_boot_ranges(early_allocated_ranges));
        return;
    };

    if get_accept_memory_mode() == AcceptMemoryMode::Eager {
        EARLY_ALLOCATED_RANGES.call_once(|| early_allocated_ranges.clone());
        let coverage = table.coverage_range();
        let coverage = coverage.start as Paddr..coverage.end as Paddr;
        add_free_ranges(
            super::allocator::free_boot_ranges(early_allocated_ranges)
                .flat_map(move |range| crate::util::ops::range_difference(&range, &coverage)),
        );
        return;
    }

    for free_range in super::allocator::free_boot_ranges(early_allocated_ranges) {
        if let Some(deferred_range) =
            try_defer_usable_memory(table, free_range.start, free_range.len())
        {
            crate::info!(
                "Deferring unaccepted frames from usable range: {:x?}",
                deferred_range
            );
            add_accepted_free_range(table, free_range.start..deferred_range.start);
            add_accepted_free_range(table, deferred_range.end..free_range.end);
        } else {
            add_accepted_free_range(table, free_range);
        }
    }
}

/// Accepts memory that must be accessed before parallel early acceptance starts.
pub(super) fn accept_early_allocated_range(start: Paddr, size: usize) {
    let Some(table) = load_unaccepted_table() else {
        return;
    };
    let start_aligned = start.align_down(crate::mm::PAGE_SIZE);
    let end_aligned = (start + size).align_up(crate::mm::PAGE_SIZE);

    // SAFETY: Before SMP startup callers are serialized; the AP stack calls operate on disjoint ranges.
    accept_boot_range(table, start_aligned..end_aligned);
}

pub(super) fn try_alloc_after_refill(layout: Layout) -> Option<Paddr> {
    crate::if_tdx_enabled!({
        let mut busy_spin_count = 0u32;
        loop {
            match super::allocator::get_global_frame_allocator().alloc(layout) {
                Some(paddr) => break Some(paddr),
                None => match try_refill_for_allocation(layout) {
                    Ok(RefillResult::Progress) => {
                        busy_spin_count = 0;
                    }
                    Ok(RefillResult::Busy) => {
                        busy_spin_count += 1;
                        for _ in 0..(1u32 << busy_spin_count.min(10)) {
                            core::hint::spin_loop();
                        }
                    }
                    Ok(RefillResult::Exhausted) => break None,
                    Err(err) => {
                        crate::warn!("Failed to refill unaccepted memory: {:?}", err);
                        break None;
                    }
                },
            }
        }
    } else {
        None
    })
}

/// Returns the total bytes of memory that remain unaccepted.
pub(super) fn load_total_unaccepted_bytes() -> usize {
    TOTAL_UNACCEPTED_BYTES.load(Ordering::Relaxed)
}

/// Attempts to defer the unit-aligned portion of a usable range that overlaps
/// unaccepted memory.
fn try_defer_usable_memory(
    table: &EfiUnacceptedMemory,
    addr: Paddr,
    size: usize,
) -> Option<Range<Paddr>> {
    if !is_range_unaccepted(table, addr, size) {
        return None;
    }

    let unit_size = table.unit_size_bytes() as usize;
    let end_addr = addr.checked_add(size)?;
    let start = addr.align_up(unit_size);
    let end = end_addr.align_down(unit_size);
    if start >= end {
        return None;
    }

    let mut remaining_range_capacity = [0; SHARD_COUNT];
    let mut initialized_shards = 0;

    // Ensure all touched shards have enough free slots to accommodate every sub-range.
    // This runs exclusively on the BSP before APs start, so shards can be
    // safely locked and unlocked individually without cross-CPU deadlock or races.
    let mut cursor = start;
    while cursor < end {
        let shard_index = shard_index_of(cursor);
        let segment_end = segment_boundary_end(cursor).min(end);
        let chunk_size = segment_end - cursor;
        let shard_bit = shard_mask(shard_index);
        let mut shard_state = DEFERRED_SHARDS[shard_index].lock();
        if initialized_shards & shard_bit == 0 {
            shard_state.compact();
            remaining_range_capacity[shard_index] = shard_state.remaining_range_capacity();
            initialized_shards |= shard_bit;
        }
        if !shard_state.can_merge_range(cursor, chunk_size) {
            if remaining_range_capacity[shard_index] == 0 {
                return None;
            }
            remaining_range_capacity[shard_index] -= 1;
        }
        cursor = segment_end;
    }

    // Insert all ranges into the reservoir and mark non-empty for runtime refill.
    let mut cursor = start;
    while cursor < end {
        let shard_index = shard_index_of(cursor);
        let segment_end = segment_boundary_end(cursor).min(end);
        let chunk_size = segment_end - cursor;

        let mut shard_state = DEFERRED_SHARDS[shard_index].lock();
        assert!(shard_state.try_push_range(cursor, chunk_size));
        mark_shard_nonempty(shard_index);
        cursor = segment_end;
    }

    Some(start..end)
}

/// Tries to refill accepted memory from the deferred unaccepted reservoir.
fn try_refill_for_allocation(layout: Layout) -> Result<RefillResult, AcceptError> {
    let Some(table) = load_unaccepted_table() else {
        return Ok(RefillResult::Exhausted);
    };
    let refill_granularity_bytes = table.unit_size_bytes() as usize;
    let request_bytes = layout
        .size()
        .max(layout.align())
        .align_up(crate::mm::PAGE_SIZE);
    let request_order_bytes = request_bytes
        .checked_next_power_of_two()
        .ok_or(AcceptError::ArithmeticOverflow)?;
    let target_chunk_bytes = request_order_bytes.max(refill_granularity_bytes);

    let accepted_bytes = accept_from_reservoir(table, target_chunk_bytes)?;
    if accepted_bytes > 0 {
        Ok(RefillResult::Progress)
    } else if has_inflight_accepts() || ACTIVE_REFILL_RESERVATIONS.load(Ordering::Acquire) != 0 {
        Ok(RefillResult::Busy)
    } else {
        Ok(RefillResult::Exhausted)
    }
}

/// Returns whether the physical range `[addr, addr + size)` may still overlap unaccepted memory.
fn is_range_unaccepted(table: &EfiUnacceptedMemory, addr: Paddr, size: usize) -> bool {
    let Some(end) = addr.checked_add(size) else {
        return true;
    };

    table.is_range_pending(addr as u64, end as u64)
}

/// Accepts the slice of memory assigned to a CPU and marks completion.
fn accept_memory_on_cpu(cpu_id: CpuId) -> Option<&'static EfiUnacceptedMemory> {
    let table = load_unaccepted_table()?;
    let num_cpus = crate::cpu::num_cpus();
    debug_assert!(cpu_id.as_usize() < num_cpus);

    let unit_size = u64::from(table.unit_size_bytes());
    let coverage_range = table.coverage_range();

    let num_units = (coverage_range.end - coverage_range.start) / unit_size;
    let (start_unit, end_unit) = partition_units(num_units, cpu_id.as_usize(), num_cpus);

    let start = coverage_range.start + start_unit * unit_size;
    let end = coverage_range.start + end_unit * unit_size;

    if start < end {
        // SAFETY: Every CPU receives a disjoint range of bitmap units.
        unsafe { table.accept_range(start, end) }.expect("failed to accept memory");
    }
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
    Some(table)
}

/// Publishes accepted memory regions to the global frame allocator.
fn publish_accepted_memory(table: &EfiUnacceptedMemory) {
    let coverage_range = table.coverage_range();
    let early_allocated_ranges = EARLY_ALLOCATED_RANGES
        .get()
        .expect("early allocated ranges are unavailable");

    let accepted_ranges =
        super::allocator::free_boot_ranges(early_allocated_ranges).filter_map(|range| {
            let start = range.start.max(coverage_range.start as Paddr);
            let end = range.end.min(coverage_range.end as Paddr);
            (start < end).then_some(start..end)
        });

    for range in accepted_ranges {
        crate::info!("Adding accepted free frames to the allocator: {:x?}", range);
        super::allocator::get_global_frame_allocator().add_free_memory(range.start, range.len());
    }
}

/// Uniformly partitions `total_units` across `num_cpus`, aligned to 8 units (1 bitmap byte).
fn partition_units(total_units: u64, index: usize, num_cpus: usize) -> (u64, u64) {
    const UNITS_PER_BYTE: u64 = 8;
    let total_bytes = total_units / UNITS_PER_BYTE;
    let base_bytes = total_bytes / num_cpus as u64;
    let remain_bytes = total_bytes % num_cpus as u64;

    let index = index as u64;
    let start_byte = base_bytes * index + remain_bytes.min(index);
    let end_byte = base_bytes * (index + 1) + remain_bytes.min(index + 1);

    let start_unit = start_byte * UNITS_PER_BYTE;
    let mut end_unit = end_byte * UNITS_PER_BYTE;
    if index == (num_cpus as u64 - 1) {
        end_unit = total_units;
    }
    (start_unit, end_unit)
}

fn add_accepted_free_range(table: &EfiUnacceptedMemory, range: Range<Paddr>) {
    if range.is_empty() {
        return;
    }
    // SAFETY: Initialization runs on the BSP before APs start, so no other
    // operation can concurrently access the acceptance bitmap.
    accept_boot_range(table, range.clone());
    add_free_ranges(core::iter::once(range));
}

fn accept_boot_range(table: &EfiUnacceptedMemory, range: Range<Paddr>) {
    let pending_before = table.pending_unit_count();
    // SAFETY: Initialization runs on the BSP before APs start, so no other
    // operation can concurrently access the acceptance bitmap.
    unsafe { table.accept_range(range.start as u64, range.end as u64) }
        .expect("failed to accept boot memory");
    let accepted_units = pending_before - table.pending_unit_count();
    TOTAL_UNACCEPTED_BYTES.fetch_sub(
        (accepted_units * u64::from(table.unit_size_bytes())) as usize,
        Ordering::Release,
    );
}

fn add_free_ranges(ranges: impl Iterator<Item = Range<Paddr>>) {
    for range in ranges {
        crate::info!("Adding free frames to the allocator: {:x?}", range);
        super::allocator::get_global_frame_allocator().add_free_memory(range.start, range.len());
    }
}

fn accept_from_reservoir(
    table: &EfiUnacceptedMemory,
    target_chunk_bytes: usize,
) -> Result<usize, AcceptError> {
    let min_chunk_bytes = table.unit_size_bytes() as usize;
    let mut accepted_total = 0usize;
    let mut remaining_budget_bytes = target_chunk_bytes + REFILL_EXTRA_BUDGET_BYTES;
    let shard_start_hint = u32::from(CpuId::current_racy()) as usize;

    while remaining_budget_bytes >= min_chunk_bytes {
        let shard_start_hint = (shard_start_hint + accepted_total / min_chunk_bytes) % SHARD_COUNT;
        let desired_chunk_bytes = target_chunk_bytes
            .min(remaining_budget_bytes)
            .align_down(min_chunk_bytes);

        let Some(mut reservation) =
            DeferredChunkReservation::try_reserve(shard_start_hint, desired_chunk_bytes)
        else {
            return Ok(accepted_total);
        };

        let reserved_bytes = reservation.size;
        let accept_result = accept_with_shard_locks(table, &mut reservation);
        let released_bytes = reserved_bytes - reservation.size;
        accepted_total += released_bytes;
        remaining_budget_bytes -= released_bytes;

        match accept_result {
            Ok(()) => {
                debug_assert_eq!(reservation.size, 0);
                reservation.commit();
            }
            Err(err) => {
                if accepted_total == 0 {
                    return Err(err);
                }
                return Ok(accepted_total);
            }
        }
    }

    Ok(accepted_total)
}

/// Accepts a GPA range using shard-level locking for multi-CPU parallelism.
fn accept_with_shard_locks(
    table: &EfiUnacceptedMemory,
    reservation: &mut DeferredChunkReservation,
) -> Result<(), AcceptError> {
    let start = reservation.addr as u64;
    let end = reservation
        .addr
        .checked_add(reservation.size)
        .ok_or(AcceptError::ArithmeticOverflow)? as u64;
    let mut cursor = start;
    let mut retry_count = 0u32;
    while cursor < end {
        let shard_index = shard_index_of(cursor as usize);
        let segment_start =
            (cursor / PHYSICAL_SEGMENT_BYTES as u64) * PHYSICAL_SEGMENT_BYTES as u64;
        let segment_end = (segment_start + PHYSICAL_SEGMENT_BYTES as u64).min(end);

        let mut retry = false;
        let claimed = {
            let mut shard = BITMAP_SHARD_LOCKS[shard_index].lock();
            if shard.num_inflight >= MAX_INFLIGHT_PER_SHARD {
                retry = true;
                None
            } else {
                // SAFETY: Shard lock guarantees exclusive access to this bitmap region.
                let run = unsafe { table.claim_next_pending_run(cursor, segment_end)? };
                if let Some((rs, re)) = run {
                    if rs < cursor || re > segment_end || rs >= re {
                        // SAFETY: The run was removed while holding the shard lock.
                        unsafe { table.restore_pending_range(rs, re) };
                        return Err(AcceptError::OutOfBounds);
                    }
                    if rs > cursor && shard.has_inflight_overlap(cursor, rs) {
                        // SAFETY: Shard lock guarantees exclusive access.
                        unsafe { table.restore_pending_range(rs, re) };
                        retry = true;
                        None
                    } else {
                        shard.add_inflight(rs, re);
                        Some((rs, re))
                    }
                } else {
                    retry = shard.has_inflight_overlap(cursor, segment_end);
                    None
                }
            }
        };

        let Some((run_start, run_end)) = claimed else {
            if retry {
                retry_count += 1;
                for _ in 0..(1u32 << retry_count.min(10)) {
                    core::hint::spin_loop();
                }
                continue;
            }
            let gap = (segment_end - cursor) as usize;
            super::allocator::get_global_frame_allocator().add_free_memory(cursor as usize, gap);
            reservation.consume_prefix_to(segment_end as usize);
            cursor = segment_end;
            continue;
        };
        retry_count = 0;

        // SAFETY: The claimed range is exclusively ours (bits cleared in bitmap + inflight registered).
        let accept_result = unsafe { accept_memory(run_start, run_end) };

        {
            let mut shard = BITMAP_SHARD_LOCKS[shard_index].lock();
            shard.remove_inflight(run_start, run_end);
            if accept_result.is_err() {
                // SAFETY: Shard lock guarantees exclusive access.
                unsafe { table.restore_pending_range(run_start, run_end) };
            }
        }

        match accept_result {
            Ok(()) => {
                let run_bytes = (run_end - run_start) as usize;
                if run_start > cursor {
                    let gap = (run_start - cursor) as usize;
                    super::allocator::get_global_frame_allocator()
                        .add_free_memory(cursor as usize, gap);
                    reservation.consume_prefix_to(run_start as usize);
                }
                super::allocator::get_global_frame_allocator()
                    .add_free_memory(run_start as usize, run_bytes);
                TOTAL_UNACCEPTED_BYTES.fetch_sub(run_bytes, Ordering::Relaxed);
                reservation.consume_prefix_to(run_end as usize);
                cursor = run_end;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

fn load_unaccepted_table() -> Option<&'static EfiUnacceptedMemory> {
    UNACCEPTED_TABLE.get().copied()
}

fn has_inflight_accepts() -> bool {
    BITMAP_SHARD_LOCKS
        .iter()
        .any(|shard| shard.lock().num_inflight != 0)
}

fn mark_shard_nonempty(shard_index: usize) {
    NONEMPTY_SHARD_HINT_BITMAP.fetch_or(shard_mask(shard_index), Ordering::Release);
}

fn mark_shard_empty(shard_index: usize) {
    NONEMPTY_SHARD_HINT_BITMAP.fetch_and(!shard_mask(shard_index), Ordering::Release);
}

const fn shard_mask(shard_index: usize) -> u64 {
    1u64 << shard_index
}

fn shard_index_of(addr: Paddr) -> usize {
    (addr / PHYSICAL_SEGMENT_BYTES) % SHARD_COUNT
}

fn segment_boundary_end(addr: Paddr) -> Paddr {
    let segment_start = (addr / PHYSICAL_SEGMENT_BYTES) * PHYSICAL_SEGMENT_BYTES;
    segment_start + PHYSICAL_SEGMENT_BYTES
}

enum RefillResult {
    Progress,
    Busy,
    Exhausted,
}

struct DeferredChunkReservation {
    shard_index: usize,
    addr: Paddr,
    size: usize,
    committed: bool,
}

impl DeferredChunkReservation {
    fn try_reserve(shard_start_hint: usize, desired_chunk: usize) -> Option<Self> {
        let nonempty_hint = NONEMPTY_SHARD_HINT_BITMAP.load(Ordering::Acquire);
        if nonempty_hint == 0 {
            return None;
        }

        for offset in 0..SHARD_COUNT {
            let shard_index = (shard_start_hint + offset) % SHARD_COUNT;
            if (nonempty_hint & shard_mask(shard_index)) == 0 {
                continue;
            }

            let mut shard_state = DEFERRED_SHARDS[shard_index].lock();
            if let Some((addr, len)) = shard_state.pop_chunk(desired_chunk) {
                ACTIVE_REFILL_RESERVATIONS.fetch_add(1, Ordering::Release);
                if shard_state.num_ranges == 0 {
                    mark_shard_empty(shard_index);
                }
                return Some(Self {
                    shard_index,
                    addr,
                    size: len,
                    committed: false,
                });
            }

            mark_shard_empty(shard_index);
        }

        None
    }

    fn commit(mut self) {
        self.committed = true;
    }

    fn consume_prefix(&mut self, size: usize) {
        debug_assert!(size <= self.size);
        self.addr += size;
        self.size -= size;
    }

    fn consume_prefix_to(&mut self, end: Paddr) {
        debug_assert!(self.addr <= end);
        self.consume_prefix(end - self.addr);
    }

    fn rollback(&self) {
        let mut shard_state = DEFERRED_SHARDS[self.shard_index].lock();
        if !shard_state.try_push_range(self.addr, self.size) {
            crate::error!(
                "reservoir rollback overflow: shard={}, addr={:#x}, size={:#x}",
                self.shard_index,
                self.addr,
                self.size
            );
            panic!("deferred chunk rollback lost physical memory");
        }
        mark_shard_nonempty(self.shard_index);
    }
}

impl Drop for DeferredChunkReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.rollback();
        }
        ACTIVE_REFILL_RESERVATIONS.fetch_sub(1, Ordering::Release);
    }
}

struct DeferredShardState {
    ranges: [DeferredRange; MAX_DEFERRED_RANGES],
    num_ranges: usize,
    pop_cursor: usize,
}

impl DeferredShardState {
    const fn new() -> Self {
        Self {
            ranges: [DeferredRange::EMPTY; MAX_DEFERRED_RANGES],
            num_ranges: 0,
            pop_cursor: 0,
        }
    }

    fn remaining_range_capacity(&self) -> usize {
        self.ranges.len() - self.num_ranges
    }

    fn can_merge_range(&self, addr: Paddr, size: usize) -> bool {
        let start = addr;
        let end = addr + size;

        self.ranges[..self.num_ranges]
            .iter()
            .any(|range| range.end == start || end == range.start)
    }

    fn try_push_range(&mut self, addr: Paddr, size: usize) -> bool {
        let start = addr;
        let end = addr + size;

        for index in 0..self.num_ranges {
            let r = &mut self.ranges[index];
            if r.end == start {
                r.end = end;
                return true;
            }
            if end == r.start {
                r.start = start;
                return true;
            }
        }

        if self.num_ranges >= self.ranges.len() * 3 / 4 {
            self.compact();
        }

        if self.num_ranges >= self.ranges.len() {
            return false;
        }

        self.ranges[self.num_ranges] = DeferredRange { start, end };
        self.num_ranges += 1;
        true
    }

    fn compact(&mut self) {
        if self.num_ranges <= 1 {
            return;
        }

        self.ranges[..self.num_ranges].sort_unstable_by_key(|r| r.start);

        let mut write = 0;
        for read in 1..self.num_ranges {
            if self.ranges[read].start <= self.ranges[write].end {
                self.ranges[write].end = self.ranges[write].end.max(self.ranges[read].end);
            } else {
                write += 1;
                self.ranges[write] = self.ranges[read];
            }
        }
        let new_len = write + 1;

        self.ranges[new_len..self.num_ranges].fill(DeferredRange::EMPTY);
        self.num_ranges = new_len;
    }

    fn pop_chunk(&mut self, target_len: usize) -> Option<(Paddr, usize)> {
        if self.num_ranges == 0 {
            return None;
        }

        let aligned_target = target_len;
        let start_cursor = self.pop_cursor % self.num_ranges;

        let index = start_cursor;
        let range = self.ranges[index];
        let chunk_len = range.len().min(aligned_target);
        let start = range.start;
        self.ranges[index].start += chunk_len;
        if self.ranges[index].is_empty() {
            self.remove_range(index);
        }
        self.pop_cursor = index + 1;
        Some((start, chunk_len))
    }

    fn remove_range(&mut self, index: usize) {
        debug_assert!(index < self.num_ranges);
        self.num_ranges -= 1;
        self.ranges[index] = self.ranges[self.num_ranges];
        self.ranges[self.num_ranges] = DeferredRange::EMPTY;
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
        self.end - self.start
    }

    const fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum AcceptMemoryMode {
    Lazy,
    Eager,
}

fn get_accept_memory_mode() -> AcceptMemoryMode {
    *ACCEPT_MEMORY_MODE.call_once(AcceptMemoryMode::from_cmdline)
}

impl AcceptMemoryMode {
    fn from_cmdline() -> Self {
        let Some(boot_info) = crate::boot::EARLY_INFO.get() else {
            return Self::Lazy;
        };

        let value = boot_info
            .kernel_cmdline
            .split_whitespace()
            .filter_map(|arg| arg.strip_prefix("accept_memory="))
            .next_back();

        match value {
            Some("eager") => Self::Eager,
            None | Some("lazy") => Self::Lazy,
            Some(v) => {
                crate::warn!("Invalid accept_memory value '{v}', using lazy");
                Self::Lazy
            }
        }
    }
}

const SHARD_COUNT: usize = 64;
const PHYSICAL_SEGMENT_BYTES: usize = 256 * 1024 * 1024;
const MAX_DEFERRED_RANGES: usize = 512;
const MAX_INFLIGHT_PER_SHARD: usize = 8;
const REFILL_EXTRA_BUDGET_BYTES: usize = 32 * 1024 * 1024;

static UNACCEPTED_TABLE: Once<&'static EfiUnacceptedMemory> = Once::new();
static EARLY_ALLOCATED_RANGES: Once<[Range<Paddr>; 2]> = Once::new();
static FINISHED_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);
static ACCEPT_MEMORY_MODE: Once<AcceptMemoryMode> = Once::new();

static TOTAL_UNACCEPTED_BYTES: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_REFILL_RESERVATIONS: AtomicUsize = AtomicUsize::new(0);

static BITMAP_SHARD_LOCKS: [SpinLock<ShardState, LocalIrqDisabled>; SHARD_COUNT] =
    [const { SpinLock::new(ShardState::new()) }; SHARD_COUNT];
static DEFERRED_SHARDS: [SpinLock<DeferredShardState, LocalIrqDisabled>; SHARD_COUNT] =
    [const { SpinLock::new(DeferredShardState::new()) }; SHARD_COUNT];
static NONEMPTY_SHARD_HINT_BITMAP: AtomicU64 = AtomicU64::new(0);
