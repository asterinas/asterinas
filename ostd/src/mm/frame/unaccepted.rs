// SPDX-License-Identifier: MPL-2.0

//! Support for unaccepted memory in confidential computing environments.

use core::{
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicPtr, Ordering},
};

use spin::Once;
use tdx_guest::{AcceptError, tdx_is_enabled, unaccepted_memory::EfiUnacceptedMemory};

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
    if !tdx_is_enabled() {
        return 0;
    }

    let Some(table_ptr) = load_unaccepted_table() else {
        return 0;
    };

    let _guard = UNACCEPTED_LOCK.lock();

    // SAFETY: `table_ptr` is initialized from boot info and points to valid table memory.
    // The UNACCEPTED_LOCK ensures exclusive mutable access to the bitmap.
    let table = unsafe { &*table_ptr.as_ptr() };
    let unit_size = u64::from(table.unit_size);

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

    let table_phys_base = table.phys_base;

    let Some(coverage_end) = table.bitmap_coverage_end() else {
        let table_size = table.size;
        let table_unit_size = table.unit_size;

        log::error!(
            "unaccepted bitmap coverage overflow: size={}, unit_size={}, phys_base={:#x}",
            table_size,
            table_unit_size,
            table_phys_base
        );
        return;
    };

    let range_start = table_phys_base;
    let range_size = coverage_end - table_phys_base;

    crate::early_println!("[kernel] Accepting all unaccepted memory ...");

    // SAFETY: The table bitmap and its physical coverage come from validated boot metadata.
    if unsafe { table.accept_by_size(range_start, range_size) }.is_ok() {
        EAGER_ACCEPT_COMPLETED.store(true, Ordering::Release);
        log::info!(
            "accept_memory=eager completed: accepted bitmap coverage [{:#x}, {:#x})",
            range_start,
            coverage_end
        );
    } else {
        log::error!(
            "accept_memory=eager failed: range=[{:#x}, {:#x})",
            range_start,
            coverage_end
        );
    }
}

/// Sets the unaccepted-memory table pointer parsed at boot entry.
pub(crate) fn set_unaccepted_memory_table(table_ptr: *mut EfiUnacceptedMemory) {
    if table_ptr.is_null() {
        log::warn!(
            "Unaccepted memory table pointer is null, lazy-accept bitmap path will be disabled"
        );
    } else {
        log::info!("Set unaccepted memory table pointer to {:p}", table_ptr);
    }
    UNACCEPTED_TABLE.store(table_ptr, Ordering::Release);
}

fn load_unaccepted_table() -> Option<NonNull<EfiUnacceptedMemory>> {
    NonNull::new(UNACCEPTED_TABLE.load(Ordering::Acquire))
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

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum AcceptMemoryMode {
    #[default]
    Lazy,
    Eager,
}

fn get_accept_memory_mode() -> AcceptMemoryMode {
    ACCEPT_MEMORY_MODE.get().copied().unwrap_or_default()
}

static UNACCEPTED_TABLE: AtomicPtr<EfiUnacceptedMemory> = AtomicPtr::new(core::ptr::null_mut());
static ACCEPT_MEMORY_MODE: Once<AcceptMemoryMode> = Once::new();
static EAGER_ACCEPT_COMPLETED: AtomicBool = AtomicBool::new(false);
static UNACCEPTED_LOCK: SpinLock<(), LocalIrqDisabled> = SpinLock::new(());
