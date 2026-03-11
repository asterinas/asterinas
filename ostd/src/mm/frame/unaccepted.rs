// SPDX-License-Identifier: MPL-2.0

//! Support for unaccepted memory in confidential computing environments.

use core::{
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicPtr, Ordering},
};

use spin::Once;
use tdx_guest::{AcceptError, unaccepted_memory::EfiUnacceptedMemory};

use crate::{
    mm::Paddr,
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

    let _guard = UNACCEPTED_LOCK.lock();

    if EAGER_ACCEPT_COMPLETED.load(Ordering::Acquire) {
        return Ok(());
    }

    let Some(table_ptr) = load_unaccepted_table() else {
        return Ok(());
    };

    let start = u64::try_from(addr).map_err(|_| AcceptError::InvalidAlignment)?;
    let len = u64::try_from(size).map_err(|_| AcceptError::InvalidAlignment)?;

    // SAFETY: `table_ptr` is initialized from boot info and points to writable bitmap memory.
    // The UNACCEPTED_LOCK ensures exclusive mutable access to the bitmap.
    unsafe { (&mut *table_ptr.as_ptr()).accept_by_size(start, len) }
}

/// Loads the total size (in bytes) of memory that is still unaccepted.
pub(crate) fn load_total_unaccepted_mem() -> usize {
    let Some(table_ptr) = load_unaccepted_table() else {
        return 0;
    };

    let _guard = UNACCEPTED_LOCK.lock();

    // SAFETY: `table_ptr` is initialized from boot info and points to valid table memory.
    // The UNACCEPTED_LOCK ensures exclusive mutable access to the bitmap.
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

    let _guard = UNACCEPTED_LOCK.lock();

    let Some(table_ptr) = load_unaccepted_table() else {
        log::warn!("accept_memory=eager requested but unaccepted table is unavailable");
        return;
    };

    // SAFETY: `table_ptr` is initialized from boot info and points to writable bitmap memory.
    // The UNACCEPTED_LOCK ensures exclusive mutable access to the bitmap.
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

    // SAFETY: The table bitmap and its physical coverage come from validated boot metadata.
    if unsafe { table.accept_range(table_phys_base, coverage_end) }.is_ok() {
        EAGER_ACCEPT_COMPLETED.store(true, Ordering::Release);
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
    // We only need to check if the chunk hits the bitmap
    let table = unsafe { &*table_ptr.as_ptr() };
    table.is_range_pending_by_size(start, len).unwrap_or(true)
}

pub(crate) fn spawn_background_accept_worker(
    spawner: impl FnOnce(alloc::boxed::Box<dyn FnOnce() + Send>),
    sleeper: impl Fn(core::time::Duration) + Send + Sync + 'static,
) {
    if get_accept_memory_mode() != AcceptMemoryMode::LazyBackground
        || BACKGROUND_WORKER_STARTED.swap(true, Ordering::AcqRel)
    {
        return;
    }
    spawner(alloc::boxed::Box::new(move || {
        background_accept_worker_loop(sleeper)
    }));
}

fn init_accept_mode_from_cmdline() {
    let mode = parse_accept_mode_from_cmdline();
    ACCEPT_MEMORY_MODE.call_once(|| mode);

    match mode {
        AcceptMemoryMode::Lazy => log::info!("accept_memory mode: lazy"),
        AcceptMemoryMode::LazyBackground => log::info!("accept_memory mode: lazy-background"),
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
        Some("lazy-background") | Some("lazy_background") => AcceptMemoryMode::LazyBackground,
        Some("eager") => AcceptMemoryMode::Eager,
        _ => {
            log::warn!("unknown accept_memory mode '{:?}', fallback to lazy", value);
            AcceptMemoryMode::Lazy
        }
    }
}

fn background_accept_worker_loop(sleeper: impl Fn(core::time::Duration)) {
    use core::{
        sync::atomic::Ordering::{Acquire, Release},
        time::Duration,
    };

    const INITIAL_CHUNK_SIZE: u64 = 32 * 1024 * 1024;
    const MIN_CHUNK_SIZE: u64 = 4 * 1024 * 1024;
    const MAX_CHUNK_SIZE: u64 = 64 * 1024 * 1024;
    const LOCK_SPIN_TRIES: u32 = 32;
    const LOCK_BACKOFF_BASE_MS: u64 = 1;
    const LOCK_BACKOFF_MAX_SHIFT: u32 = 3;
    const ITERATION_SLEEP_MS: u64 = 5;

    if get_accept_memory_mode() != AcceptMemoryMode::LazyBackground {
        return;
    }

    log::info!("lazy-background accept worker started");

    // Allow kernel and user space to fully boot before starting.
    sleeper(Duration::from_secs(3));

    let Some(table_ptr) = load_unaccepted_table() else {
        return;
    };

    // SAFETY: table_ptr is valid while UNACCEPTED_TABLE holds it.
    let table = unsafe { &*table_ptr.as_ptr() };
    let Some(end) = table.bitmap_coverage_end() else {
        return;
    };

    let mut cursor = table.phys_base();
    let mut chunk_size = INITIAL_CHUNK_SIZE;
    let mut lock_miss_count: u32 = 0;
    let mut consecutive_successes: u32 = 0;

    let sleep = |ms| sleeper(Duration::from_millis(ms));

    while cursor < end {
        if EAGER_ACCEPT_COMPLETED.load(Acquire) || BACKGROUND_WORKER_DISABLED.load(Acquire) {
            break;
        }

        let Some(guard) = UNACCEPTED_LOCK.try_lock() else {
            // Lock contention: back off and retry.
            lock_miss_count = lock_miss_count.saturating_add(1);
            consecutive_successes = 0;

            for _ in 0..LOCK_SPIN_TRIES {
                core::hint::spin_loop();
            }

            if lock_miss_count >= 8 {
                chunk_size = (chunk_size / 2).max(MIN_CHUNK_SIZE);
            }

            let shift = lock_miss_count
                .saturating_sub(1)
                .min(LOCK_BACKOFF_MAX_SHIFT);
            sleep(LOCK_BACKOFF_BASE_MS << shift);
            continue;
        };

        // Re-check after acquiring lock.
        if EAGER_ACCEPT_COMPLETED.load(Acquire) {
            break;
        }

        let current_end = cursor.saturating_add(chunk_size).min(end);
        let chunk_len = current_end - cursor;

        // SAFETY: lock guarantees exclusive mutable access to the table.
        let table = unsafe { &mut *table_ptr.as_ptr() };

        let is_pending = match table.is_range_pending_by_size(cursor, chunk_len) {
            Ok(pending) => pending,
            Err(err) => {
                log::error!(
                    "Background accept pending-check failed at {:#x}: {:?}",
                    cursor,
                    err
                );
                BACKGROUND_WORKER_DISABLED.store(true, Release);
                break;
            }
        };

        if is_pending {
            // SAFETY: lock guarantees exclusive mutable access.
            if let Err(err) = unsafe { table.accept_by_size(cursor, chunk_len) } {
                log::error!("Background accept failed at {:#x}: {:?}", cursor, err);
                BACKGROUND_WORKER_DISABLED.store(true, Release);
                break;
            }
            consecutive_successes = consecutive_successes.saturating_add(1);
            if consecutive_successes >= 4 {
                chunk_size = chunk_size.saturating_mul(2).min(MAX_CHUNK_SIZE);
                consecutive_successes = 0;
            }
        }

        cursor = current_end;
        lock_miss_count = 0;

        // Release the lock before sleeping to re-enable IRQs.
        drop(guard);
        sleep(ITERATION_SLEEP_MS);
    }

    if cursor >= end && !BACKGROUND_WORKER_DISABLED.load(Acquire) {
        EAGER_ACCEPT_COMPLETED.store(true, Release);
        log::info!("lazy-background accept worker finished normally");
    } else if BACKGROUND_WORKER_DISABLED.load(Acquire) {
        log::warn!("lazy-background accept worker stopped: background refill disabled");
    }
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum AcceptMemoryMode {
    #[default]
    Lazy,
    LazyBackground,
    Eager,
}

fn load_unaccepted_table() -> Option<NonNull<EfiUnacceptedMemory>> {
    NonNull::new(UNACCEPTED_TABLE.load(Ordering::Acquire))
}

fn get_accept_memory_mode() -> AcceptMemoryMode {
    ACCEPT_MEMORY_MODE.get().copied().unwrap_or_default()
}

static UNACCEPTED_TABLE: AtomicPtr<EfiUnacceptedMemory> = AtomicPtr::new(core::ptr::null_mut());
static ACCEPT_MEMORY_MODE: Once<AcceptMemoryMode> = Once::new();
static EAGER_ACCEPT_COMPLETED: AtomicBool = AtomicBool::new(false);
static UNACCEPTED_LOCK: SpinLock<(), LocalIrqDisabled> = SpinLock::new(());
static BACKGROUND_WORKER_STARTED: AtomicBool = AtomicBool::new(false);
static BACKGROUND_WORKER_DISABLED: AtomicBool = AtomicBool::new(false);
