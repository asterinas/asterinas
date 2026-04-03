// SPDX-License-Identifier: MPL-2.0

//! Support for unaccepted memory in confidential computing environments.

use core::{
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use spin::Once;
use tdx_guest::{
    AcceptError,
    unaccepted_memory::{BitmapSegmentLock, BitmapSegmentLocks, EfiUnacceptedMemory},
};

use crate::{
    irq,
    mm::{PAGE_SIZE, Paddr},
    sync::{LocalIrqDisabled, SpinLock},
};

/// Initializes unaccepted-memory support from the boot-time parsed table pointer.
pub(crate) fn init() {
    init_accept_mode_from_cmdline();

    if load_unaccepted_table().is_some() {
        try_eager_accept_memory();
    } else {
        log::warn!("Unaccepted memory table is unavailable; fallback accept path will be used");
    }
}

/// Defers a usable range that still overlaps unaccepted memory.
///
/// Returns `true` if the range is deferred to the unaccepted reservoir.
pub(crate) fn defer_usable_memory(addr: Paddr, size: usize) -> bool {
    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) || !is_range_unaccepted(addr, size) {
        return false;
    }

    let start = addr.align_up(PAGE_SIZE);
    let end = addr.saturating_add(size).align_down(PAGE_SIZE);
    if start >= end {
        return true;
    }

    let mut cursor = start;
    while cursor < end {
        let segment_index = segment_index_of(cursor);
        let segment_end = segment_boundary_end(cursor).min(end);
        let chunk_size = segment_end - cursor;

        let mut segment_state = SEGMENT_STATES[segment_index].lock();
        if !segment_state.push_range(cursor, chunk_size) {
            return false;
        }
        cursor = segment_end;
    }

    TOTAL_DEFERRED_BYTES.fetch_add(end - start, Ordering::Relaxed);
    true
}

/// Refills accepted memory from the deferred unaccepted reservoir.
///
/// Returns `Ok(true)` if at least one chunk is accepted and released into
/// the global frame allocator.
pub(crate) fn refill_for_allocation(
    layout: core::alloc::Layout,
    force: bool,
) -> Result<bool, AcceptError> {
    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return Ok(false);
    }

    let mut target_chunk = layout.size().max(layout.align()).align_up(PAGE_SIZE);
    if !force {
        target_chunk = target_chunk.min(DEFAULT_REFILL_CHUNK_BYTES);
    }
    target_chunk = target_chunk.max(PAGE_SIZE);

    let budget = if force {
        target_chunk.saturating_add(FORCED_REFILL_EXTRA_BYTES)
    } else {
        DEFAULT_REFILL_BUDGET_BYTES
    };

    let accepted_bytes = refill_deferred_memory(target_chunk, budget)?;
    Ok(accepted_bytes > 0)
}

/// Rewrites the EFI-table pointer to the kernel linear mapping after the kernel page table
/// has been activated.
pub(crate) fn remap_table_ptr_after_paging() {
    let Some(old_ptr) = load_unaccepted_table() else {
        return;
    };

    let old_addr = old_ptr.as_ptr().addr();
    if old_addr < crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR {
        let new_ptr = crate::mm::kspace::paddr_to_vaddr(old_addr) as *mut EfiUnacceptedMemory;
        UNACCEPTED_TABLE.store(new_ptr, Ordering::Release);
        log::info!(
            "Remapped unaccepted memory table pointer: {:#x} -> {:#x}",
            old_addr,
            new_ptr.addr()
        );
    }
}

/// Accepts memory on demand if the target range is still unaccepted.
pub(crate) fn accept_memory_if_needed(addr: Paddr, size: usize) -> Result<(), AcceptError> {
    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return Ok(());
    }

    let Some(table_ptr) = load_unaccepted_table() else {
        return Ok(());
    };

    let start = u64::try_from(addr).map_err(|_| AcceptError::InvalidAlignment)?;
    let len = u64::try_from(size).map_err(|_| AcceptError::InvalidAlignment)?;
    let end = start
        .checked_add(len)
        .ok_or(AcceptError::ArithmeticOverflow)?;

    // Build lock shards from bitmap size so independent bit ranges can proceed concurrently.
    let table = unsafe { &mut *table_ptr.as_ptr() };
    let locks = bitmap_segment_locks_for(table)?;
    let _irq_guard = irq::disable_local();

    // SAFETY: `table_ptr` is initialized from boot info and points to writable bitmap memory.
    unsafe { table.accept_range(start, end, &locks) }
}

/// Loads the total size (in bytes) of memory that is still unaccepted.
pub(crate) fn load_total_unaccepted_mem() -> usize {
    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return 0;
    }

    let deferred = TOTAL_DEFERRED_BYTES.load(Ordering::Relaxed);
    if deferred > 0 {
        return deferred;
    }

    let Some(table_ptr) = load_unaccepted_table() else {
        return 0;
    };

    // SAFETY: `table_ptr` is initialized from boot info and points to valid table memory.
    let table = unsafe { &*table_ptr.as_ptr() };
    let unit_size = u64::from(table.unit_size_bytes());

    // SAFETY: The table and bitmap memory are valid while referenced by `UNACCEPTED_TABLE`.
    let bitmap = unsafe { table.as_bitmap_slice() };
    let total_unaccepted_bits: u64 = bitmap
        .iter()
        .map(|byte| u64::from(byte.count_ones()))
        .sum::<u64>();

    let total_unaccepted_bytes = total_unaccepted_bits.saturating_mul(unit_size);
    usize::try_from(total_unaccepted_bytes)
        .expect("total unaccepted memory should fit in usize on this target")
}

pub(crate) fn try_eager_accept_memory() {
    if get_accept_memory_mode() != AcceptMemoryMode::Eager {
        return;
    }

    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return;
    }

    let Some(table_ptr) = load_unaccepted_table() else {
        log::warn!("accept_memory=eager requested but unaccepted table is unavailable");
        return;
    };

    // SAFETY: `table_ptr` is initialized from boot info and points to writable bitmap memory.
    let table = unsafe { &mut *table_ptr.as_ptr() };

    let table_phys_base = table.phys_base();

    let Some(coverage_end) = table.bitmap_coverage_end() else {
        let table_bitmap_size_bytes = table.bitmap_size_bytes();
        let table_unit_size_bytes = table.unit_size_bytes();

        log::error!(
            "unaccepted bitmap coverage overflow: bitmap_size_bytes={}, unit_size_bytes={}, phys_base={:#x}",
            table_bitmap_size_bytes,
            table_unit_size_bytes,
            table_phys_base
        );
        return;
    };

    crate::early_println!("[kernel] Accepting all unaccepted memory ...");

    let locks = match bitmap_segment_locks_for(table) {
        Ok(locks) => locks,
        Err(err) => {
            log::error!(
                "accept_memory=eager failed to initialize bitmap locks: {:?}",
                err
            );
            return;
        }
    };
    let _irq_guard = irq::disable_local();

    // SAFETY: The table bitmap and its physical coverage come from validated boot metadata.
    if unsafe { table.accept_range(table_phys_base, coverage_end, &locks) }.is_ok() {
        EAGER_ACCEPT_COMPLETED.store(true, Ordering::Release);
        clear_deferred_ranges();
        log::info!(
            "accept_memory=eager completed: accepted bitmap coverage [{:#x}, {:#x})",
            table_phys_base,
            coverage_end
        );
    } else {
        log::error!(
            "accept_memory=eager failed: range=[{:#x}, {:#x})",
            table_phys_base,
            coverage_end
        );
    }
}

/// Sets the unaccepted-memory table pointer parsed at boot entry.
pub(crate) fn set_unaccepted_memory_table(table: Option<&'static mut EfiUnacceptedMemory>) {
    match table {
        Some(t) => {
            log::info!("Set unaccepted memory table pointer to {:p}", t);
            UNACCEPTED_TABLE.store(t as *mut _, Ordering::Release);
        }
        None => {
            log::warn!(
                "Unaccepted memory table is unavailable, lazy-accept bitmap path will be disabled"
            );
        }
    }
}

pub(crate) fn is_range_unaccepted(addr: crate::mm::Paddr, size: usize) -> bool {
    if EAGER_ACCEPT_COMPLETED.load(core::sync::atomic::Ordering::Acquire) {
        return false;
    }
    let Ok(start) = u64::try_from(addr) else {
        return true;
    };
    let Ok(len) = u64::try_from(size) else {
        return true;
    };

    let Some(table_ptr) = load_unaccepted_table() else {
        return false;
    };

    let table = unsafe { &*table_ptr.as_ptr() };
    let Some(end) = start.checked_add(len) else {
        return true;
    };

    let Ok(locks) = bitmap_segment_locks_for(table) else {
        return true;
    };

    let _irq_guard = irq::disable_local();

    table.is_range_pending(start, end, &locks).unwrap_or(true)
}

fn init_accept_mode_from_cmdline() {
    let mode = parse_accept_mode_from_cmdline();
    ACCEPT_MEMORY_MODE.call_once(|| mode);

    match mode {
        AcceptMemoryMode::Lazy => log::info!("accept_memory mode: lazy"),
        AcceptMemoryMode::Eager => log::info!("accept_memory mode: eager"),
    }
}

fn parse_accept_mode_from_cmdline() -> AcceptMemoryMode {
    let Some(boot_info) = crate::boot::EARLY_INFO.get() else {
        return AcceptMemoryMode::Lazy;
    };

    let value = boot_info
        .kernel_cmdline
        .split_whitespace()
        .find(|arg| arg.starts_with("accept_memory="))
        .and_then(|arg| arg.split_once('='))
        .map(|(_, value)| value);

    match value {
        Some("lazy") => AcceptMemoryMode::Lazy,
        Some("eager") => AcceptMemoryMode::Eager,
        _ => {
            log::warn!("unknown accept_memory mode '{:?}', fallback to lazy", value);
            AcceptMemoryMode::Lazy
        }
    }
}

fn load_unaccepted_table() -> Option<NonNull<EfiUnacceptedMemory>> {
    NonNull::new(UNACCEPTED_TABLE.load(Ordering::Acquire))
}

fn get_accept_memory_mode() -> AcceptMemoryMode {
    ACCEPT_MEMORY_MODE.get().copied().unwrap_or_default()
}

fn is_reservoir_empty() -> bool {
    TOTAL_DEFERRED_BYTES.load(Ordering::Relaxed) == 0
}

fn refill_deferred_memory(target_chunk: usize, budget: usize) -> Result<usize, AcceptError> {
    let mut accepted_total = 0usize;
    let mut remaining_budget = budget.max(PAGE_SIZE);
    let seed = current_cpu_seed();

    while remaining_budget >= PAGE_SIZE {
        if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
            return Ok(accepted_total);
        }

        let desired_chunk = target_chunk
            .min(remaining_budget)
            .max(PAGE_SIZE)
            .align_up(PAGE_SIZE);

        let Some((segment_index, chunk_addr, chunk_len)) =
            reserve_deferred_chunk(seed, desired_chunk)
        else {
            return Ok(accepted_total);
        };

        let accepted = accept_chunk_via_bitmap_table(chunk_addr, chunk_len);
        match accepted {
            Ok(()) => {
                TOTAL_DEFERRED_BYTES.fetch_sub(chunk_len, Ordering::Relaxed);
                super::allocator::get_global_frame_allocator()
                    .add_free_memory(chunk_addr, chunk_len);
                accepted_total = accepted_total.saturating_add(chunk_len);
                remaining_budget = remaining_budget.saturating_sub(chunk_len);
            }
            Err(err) => {
                return_deferred_chunk(segment_index, chunk_addr, chunk_len);
                return Err(err);
            }
        }
    }

    Ok(accepted_total)
}

fn accept_chunk_via_bitmap_table(addr: Paddr, len: usize) -> Result<(), AcceptError> {
    let Some(table_ptr) = load_unaccepted_table() else {
        return Ok(());
    };

    let start = u64::try_from(addr).map_err(|_| AcceptError::InvalidAlignment)?;
    let len = u64::try_from(len).map_err(|_| AcceptError::InvalidAlignment)?;
    let end = start
        .checked_add(len)
        .ok_or(AcceptError::ArithmeticOverflow)?;

    let table = unsafe { &mut *table_ptr.as_ptr() };
    let locks = bitmap_segment_locks_for(table)?;
    let _irq_guard = irq::disable_local();

    // SAFETY: `table_ptr` is initialized from boot info and points to writable bitmap memory.
    unsafe { table.accept_range(start, end, &locks) }
}

fn bitmap_segment_locks_for(
    table: &EfiUnacceptedMemory,
) -> Result<BitmapSegmentLocks<'static>, AcceptError> {
    let total_bits = table
        .bitmap_size_bytes()
        .checked_mul(8)
        .ok_or(AcceptError::ArithmeticOverflow)?;
    let shard_count =
        u64::try_from(BITMAP_LOCK_SHARD_COUNT).map_err(|_| AcceptError::OutOfBounds)?;

    let bits_per_lock = if total_bits == 0 {
        1
    } else {
        total_bits
            .checked_add(shard_count - 1)
            .ok_or(AcceptError::ArithmeticOverflow)?
            / shard_count
    };

    BitmapSegmentLocks::new(&UNACCEPTED_BITMAP_SEGMENT_LOCKS, bits_per_lock)
}

fn reserve_deferred_chunk(seed: usize, desired_chunk: usize) -> Option<(usize, Paddr, usize)> {
    for offset in 0..SEGMENT_LOCK_COUNT {
        let segment_index = (seed + offset) % SEGMENT_LOCK_COUNT;
        let mut segment_state = SEGMENT_STATES[segment_index].lock();
        if let Some((addr, len)) = segment_state.pop_chunk(desired_chunk) {
            return Some((segment_index, addr, len));
        }
    }

    None
}

fn return_deferred_chunk(segment_index: usize, addr: Paddr, size: usize) {
    let mut segment_state = SEGMENT_STATES[segment_index].lock();
    let inserted = segment_state.push_range(addr, size);
    debug_assert!(inserted, "segment reservoir should have room for rollback");
}

fn current_cpu_seed() -> usize {
    NEXT_SEED.fetch_add(1, Ordering::Relaxed) % SEGMENT_LOCK_COUNT
}

fn clear_deferred_ranges() {
    for segment_index in 0..SEGMENT_LOCK_COUNT {
        let mut segment_state = SEGMENT_STATES[segment_index].lock();
        segment_state.clear();
    }
    TOTAL_DEFERRED_BYTES.store(0, Ordering::Relaxed);
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

struct SegmentState {
    ranges: [DeferredRange; MAX_DEFERRED_RANGES],
    nranges: usize,
}

impl SegmentState {
    const fn new() -> Self {
        Self {
            ranges: [DeferredRange::EMPTY; MAX_DEFERRED_RANGES],
            nranges: 0,
        }
    }

    fn push_range(&mut self, addr: Paddr, size: usize) -> bool {
        if self.nranges >= self.ranges.len() {
            return false;
        }

        let start = addr.align_up(PAGE_SIZE);
        let end = addr.saturating_add(size).align_down(PAGE_SIZE);
        if start >= end {
            return true;
        }

        self.ranges[self.nranges] = DeferredRange { start, end };
        self.nranges += 1;
        true
    }

    fn pop_chunk(&mut self, target_len: usize) -> Option<(Paddr, usize)> {
        let aligned_target = target_len.max(PAGE_SIZE).align_up(PAGE_SIZE);
        for index in 0..self.nranges {
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
                return Some((start, chunk_len));
            }
        }

        None
    }

    fn remove_range(&mut self, index: usize) {
        debug_assert!(index < self.nranges);
        self.nranges -= 1;
        self.ranges[index] = self.ranges[self.nranges];
        self.ranges[self.nranges] = DeferredRange::EMPTY;
    }

    fn clear(&mut self) {
        self.nranges = 0;
        self.ranges.fill(DeferredRange::EMPTY);
    }
}

fn segment_index_of(addr: Paddr) -> usize {
    (addr / SEGMENT_BYTES) % SEGMENT_LOCK_COUNT
}

fn segment_boundary_end(addr: Paddr) -> Paddr {
    let segment_start = (addr / SEGMENT_BYTES) * SEGMENT_BYTES;
    segment_start.saturating_add(SEGMENT_BYTES)
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum AcceptMemoryMode {
    #[default]
    Lazy,
    Eager,
}

const MAX_DEFERRED_RANGES: usize = 2048;
const SEGMENT_LOCK_COUNT: usize = 64;
const BITMAP_LOCK_SHARD_COUNT: usize = 64;
const SEGMENT_BYTES: usize = 256 * 1024 * 1024;
const DEFAULT_REFILL_CHUNK_BYTES: usize = 2 * 1024 * 1024;
const FORCED_REFILL_EXTRA_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_REFILL_BUDGET_BYTES: usize = 64 * 1024 * 1024;

static UNACCEPTED_TABLE: AtomicPtr<EfiUnacceptedMemory> = AtomicPtr::new(core::ptr::null_mut());
static ACCEPT_MEMORY_MODE: Once<AcceptMemoryMode> = Once::new();
static EAGER_ACCEPT_COMPLETED: AtomicBool = AtomicBool::new(false);
static UNACCEPTED_BITMAP_SEGMENT_LOCKS: [BitmapSegmentLock; BITMAP_LOCK_SHARD_COUNT] =
    [const { BitmapSegmentLock::new(false) }; BITMAP_LOCK_SHARD_COUNT];
static SEGMENT_STATES: [SpinLock<SegmentState, LocalIrqDisabled>; SEGMENT_LOCK_COUNT] =
    [const { SpinLock::new(SegmentState::new()) }; SEGMENT_LOCK_COUNT];
static TOTAL_DEFERRED_BYTES: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEED: AtomicUsize = AtomicUsize::new(0);
