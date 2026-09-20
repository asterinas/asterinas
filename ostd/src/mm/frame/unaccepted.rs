// SPDX-License-Identifier: MPL-2.0

//! Unaccepted-memory management for Intel TDX guests.
//!
//! # Background
//!
//! Under Intel Trust Domain Extensions (TDX), physical memory assigned to the guest is initially
//! marked as *unaccepted*. To make physical memory usable, the kernel must explicitly accept it.
//!
//! # Architecture & Strategies
//!
//! This module supports two memory acceptance modes:
//!
//! 1. **Eager Mode**: Memory is partitioned evenly across all available CPUs during early boot,
//!    allowing APs to accept the entire physical address space in parallel before userspace starts.
//! 2. **Lazy Mode (Default)**: Accepting gigabytes of memory eagerly incurs severe boot latency.
//!    Usable memory is published to the frame allocator immediately. Each allocation is accepted
//!    on demand before being returned to the caller or its frames are initialized or accessed.

use core::{
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use linux_boot_params::{BootParams, EfiInfo};
use spin::Once;
use tdx_guest::{AcceptError, accept_memory, unaccepted_memory::EfiUnacceptedMemory};

use crate::{
    boot::AcceptMemoryMode,
    cpu::CpuId,
    mm::Paddr,
    sync::{LocalIrqDisabled, SpinLock},
    util::id_set::Id,
};

/// Initializes the unaccepted-memory table from the EFI stub boot parameters.
pub(crate) fn init(boot_params: &BootParams) {
    let efi_info = boot_params.efi_info;
    assert_eq!(
        efi_info.efi_loader_signature,
        EfiInfo::ASTERINAS_LOADER_SIGNATURE,
        "TDX guests must be booted by the Asterinas EFI stub"
    );
    assert_eq!(
        efi_info.efi_systab_hi, 0,
        "Asterinas unaccepted-memory table must be below 4 GiB"
    );

    let table_addr = efi_info.efi_systab;
    if table_addr == 0 {
        return;
    }
    let table_ptr =
        crate::mm::kspace::paddr_to_vaddr(table_addr as usize) as *const EfiUnacceptedMemory;

    crate::info!("Found unaccepted memory table at {:p}", table_ptr);
    // SAFETY: The EFI stub provides a valid table that remains alive for the kernel lifetime.
    let table: &'static EfiUnacceptedMemory = unsafe { &*table_ptr };
    let unit_size = u64::from(table.unit_size_bytes());
    if unit_size == 0 {
        crate::warn!("Ignoring unaccepted memory table with zero unit size");
        return;
    }
    let total_bits = table.pending_unit_count();
    TOTAL_UNACCEPTED_BYTES.store((total_bits * unit_size) as usize, Ordering::Release);

    UNACCEPTED_TABLE.call_once(|| table);
}

/// Returns the physical range occupied by the table storage.
pub(crate) fn table_memory_range() -> Option<Range<Paddr>> {
    let table = UNACCEPTED_TABLE.get().copied()?;
    let base = core::ptr::from_ref(table).addr() - crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR;
    let size_bytes = size_of::<EfiUnacceptedMemory>() + table.bitmap_size_bytes() as usize;
    Some(base..base + size_bytes)
}

/// Accepts the BSP's slice of unaccepted memory, waits for all APs to finish,
/// and publishes the accepted memory to the global frame allocator.
///
/// # Safety
///
/// This function must be called only once in the boot context of the BSP, after booting APs.
pub(crate) unsafe fn accept_memory_on_bsp() {
    if crate::boot::early_cmdline().accept_memory_mode != AcceptMemoryMode::Eager {
        return;
    }

    let Some(table) = UNACCEPTED_TABLE.get().copied() else {
        return;
    };

    let num_cpus = crate::cpu::num_cpus();
    let range = range_to_accept_on_cpu(table, CpuId::bsp().as_usize(), num_cpus);
    if !range.is_empty() {
        // SAFETY:
        // - This table and bitmap describe pending private-memory units.
        // - The BSP and APs accept pages concurrently, but their ranges are disjoint.
        unsafe { table.accept_range(range.start, range.end) }.expect("failed to accept memory");
    }
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);

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
/// # Safety
///
/// This function must be called only once per AP in its boot context.
pub(crate) unsafe fn accept_memory_on_ap() {
    if crate::boot::early_cmdline().accept_memory_mode != AcceptMemoryMode::Eager {
        return;
    }

    let Some(table) = UNACCEPTED_TABLE.get().copied() else {
        return;
    };

    let num_cpus = crate::cpu::num_cpus();
    let range = range_to_accept_on_cpu(table, CpuId::current_racy().as_usize(), num_cpus);
    if !range.is_empty() {
        // SAFETY:
        // - This table and bitmap describe pending private-memory units.
        // - The BSP and APs accept pages concurrently, but their ranges are disjoint.
        unsafe { table.accept_range(range.start, range.end) }.expect("failed to accept memory");
    }
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
}

/// Records the early-allocated ranges and returns the unaccepted-memory coverage range.
/// It may include memory that has already been accepted.
pub(super) fn init_unaccepted_memory_state(
    early_allocated_ranges: &[Range<Paddr>; 2],
) -> Range<Paddr> {
    if crate::boot::early_cmdline().accept_memory_mode != AcceptMemoryMode::Eager {
        return 0..0;
    }

    let Some(table) = UNACCEPTED_TABLE.get().copied() else {
        return 0..0;
    };
    EARLY_ALLOCATED_RANGES.call_once(|| early_allocated_ranges.clone());

    let coverage_range = table.coverage_range();
    coverage_range.start as Paddr..coverage_range.end as Paddr
}

/// Accepts memory that must be accessed before parallel early acceptance starts.
///
/// # Safety
///
/// This function must not be called concurrently with any other functions that accept pages.
pub(super) unsafe fn accept_early_allocated_range(start: Paddr, size: usize) {
    let Some(table) = UNACCEPTED_TABLE.get().copied() else {
        return;
    };

    let pending_before = table.pending_unit_count();
    let start_aligned = start.align_down(crate::mm::PAGE_SIZE);
    let end_aligned = (start + size).align_up(crate::mm::PAGE_SIZE);

    // SAFETY:
    // - This table and bitmap describe pending private-memory units.
    // - The caller guarantees no concurrent acceptance operation.
    unsafe { table.accept_range(start_aligned as u64, end_aligned as u64) }
        .expect("failed to accept boot memory");

    let accepted_units = pending_before - table.pending_unit_count();
    let accepted_bytes = (accepted_units * u64::from(table.unit_size_bytes())) as usize;
    TOTAL_UNACCEPTED_BYTES.fetch_sub(accepted_bytes, Ordering::Release);
}

/// Accepts bitmap units overlapping an allocated range before the caller accesses its pages.
pub(super) fn ensure_accepted(start: Paddr, size_bytes: usize) -> Result<(), AcceptError> {
    let Some(table) = UNACCEPTED_TABLE.get().copied() else {
        return Ok(());
    };

    let end = start
        .checked_add(size_bytes)
        .ok_or(AcceptError::ArithmeticOverflow)?;
    let unit_size = table.unit_size_bytes() as usize;
    let mut cursor = start.align_down(unit_size);
    let end = end.align_up(unit_size);
    let mut retry_count = 0u32;

    while cursor < end {
        let shard_index = shard_index_of(cursor);
        let segment_end = segment_boundary_end(cursor).min(end);
        let claim_outcome = {
            let mut shard = BITMAP_SHARD_LOCKS[shard_index].lock();
            if shard.num_inflight >= MAX_INFLIGHT_PER_SHARD
                || shard.has_inflight_overlap(cursor as u64, segment_end as u64)
            {
                ClaimOutcome::Retry
            } else {
                // SAFETY: The shard lock serializes claims for this physical segment.
                let run =
                    unsafe { table.claim_next_pending_run(cursor as u64, segment_end as u64)? };
                if let Some((run_start, run_end)) = run {
                    if run_start < cursor as u64
                        || run_end > segment_end as u64
                        || run_start >= run_end
                    {
                        // SAFETY: The range was claimed while holding the shard lock.
                        unsafe { table.restore_pending_range(run_start, run_end) };
                        return Err(AcceptError::OutOfBounds);
                    }
                    shard.add_inflight(run_start, run_end);
                    ClaimOutcome::Claimed(run_start..run_end)
                } else {
                    ClaimOutcome::NoPending
                }
            }
        };

        let (run_start, run_end) = match claim_outcome {
            ClaimOutcome::Claimed(range) => {
                retry_count = 0;
                (range.start, range.end)
            }
            ClaimOutcome::Retry => {
                retry_count += 1;
                for _ in 0..(1u32 << retry_count.min(10)) {
                    core::hint::spin_loop();
                }
                continue;
            }
            ClaimOutcome::NoPending => {
                cursor = segment_end;
                retry_count = 0;
                continue;
            }
        };

        // SAFETY: The pending bits are claimed and the in-flight range is registered.
        if let Err(err) = unsafe { accept_memory(run_start, run_end) } {
            // `accept_memory` may accept a prefix before returning an error, but does not report
            // that progress. Keep the range claimed and fail-stop rather than restoring bitmap
            // bits that may describe already-accepted memory or returning it to the allocator.
            panic!(
                "failed to accept memory {:#x}..{:#x}: {:?}",
                run_start, run_end, err
            );
        }

        {
            let mut shard = BITMAP_SHARD_LOCKS[shard_index].lock();
            shard.remove_inflight(run_start, run_end);
        }

        TOTAL_UNACCEPTED_BYTES.fetch_sub((run_end - run_start) as usize, Ordering::Relaxed);
        cursor = run_end as usize;
    }

    Ok(())
}

/// Returns the total bytes of memory that remain unaccepted.
pub(super) fn load_total_unaccepted_bytes() -> usize {
    TOTAL_UNACCEPTED_BYTES.load(Ordering::Relaxed)
}

/// Publishes accepted memory regions to the global frame allocator.
fn publish_accepted_memory(table: &EfiUnacceptedMemory) {
    let coverage_range = table.coverage_range();
    let early_allocated_ranges = EARLY_ALLOCATED_RANGES
        .get()
        .expect("early allocated ranges are unavailable");

    let accepted_ranges = super::allocator::usable_boot_ranges_excluding(early_allocated_ranges)
        .filter_map(|range| {
            let start = range.start.max(coverage_range.start as Paddr);
            let end = range.end.min(coverage_range.end as Paddr);
            (start < end).then_some(start..end)
        });

    for range in accepted_ranges {
        crate::info!("Adding accepted free frames to the allocator: {:x?}", range);
        super::allocator::get_global_frame_allocator().add_free_memory(range.start, range.len());
    }
}

/// Returns the GPA range assigned to one CPU within the table coverage.
fn range_to_accept_on_cpu(
    table: &EfiUnacceptedMemory,
    index: usize,
    num_cpus: usize,
) -> Range<u64> {
    debug_assert!(index < num_cpus);

    let coverage_range = table.coverage_range();
    let unit_size = u64::from(table.unit_size_bytes());
    let num_units = (coverage_range.end - coverage_range.start) / unit_size;
    let (start_unit, end_unit) = partition_units(num_units, index, num_cpus);

    let start = coverage_range.start + start_unit * unit_size;
    let end = coverage_range.start + end_unit * unit_size;
    start..end
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

fn shard_index_of(addr: Paddr) -> usize {
    (addr / PHYSICAL_SEGMENT_BYTES) % SHARD_COUNT
}

fn segment_boundary_end(addr: Paddr) -> Paddr {
    let segment_start = (addr / PHYSICAL_SEGMENT_BYTES) * PHYSICAL_SEGMENT_BYTES;
    segment_start + PHYSICAL_SEGMENT_BYTES
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

enum ClaimOutcome {
    Claimed(Range<u64>),
    Retry,
    NoPending,
}

const SHARD_COUNT: usize = 64;
const PHYSICAL_SEGMENT_BYTES: usize = 256 * 1024 * 1024;
const MAX_INFLIGHT_PER_SHARD: usize = 8;

static UNACCEPTED_TABLE: Once<&'static EfiUnacceptedMemory> = Once::new();
static EARLY_ALLOCATED_RANGES: Once<[Range<Paddr>; 2]> = Once::new();
static FINISHED_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);

static TOTAL_UNACCEPTED_BYTES: AtomicUsize = AtomicUsize::new(0);

static BITMAP_SHARD_LOCKS: [SpinLock<ShardState, LocalIrqDisabled>; SHARD_COUNT] =
    [const { SpinLock::new(ShardState::new()) }; SHARD_COUNT];
