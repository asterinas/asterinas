// SPDX-License-Identifier: MPL-2.0

//! Address Space ID (ASID) allocation.
//!
//! This module provides functions to allocate and deallocate ASIDs.

use core::sync::atomic::{AtomicU16, Ordering};

use log;

extern crate alloc;
use alloc::collections::{btree_map::Entry, BTreeMap};

/// The maximum ASID value from the architecture.
///
/// When we run out of ASIDs, we use this special value to indicate
/// that the TLB entries for this address space need to be flushed
/// using INVPCID on context switch.
pub use crate::arch::mm::ASID_CAP;
use crate::sync::SpinLock;

/// The special ASID value that indicates the TLB entries for this
/// address space need to be flushed on context switch.
pub const ASID_FLUSH_REQUIRED: u16 = ASID_CAP;

/// The lowest ASID value that can be allocated.
///
/// ASID 0 is typically reserved for the kernel.
pub const ASID_MIN: u16 = 1;

/// Global ASID allocator.
static ASID_ALLOCATOR: SpinLock<AsidAllocator> = SpinLock::new(AsidAllocator::new());

/// Global map of ASID to generation
static ASID_MAP: SpinLock<BTreeMap<u16, u16>> = SpinLock::new(BTreeMap::new());

/// Current ASID generation
static ASID_GENERATION: AtomicU16 = AtomicU16::new(0);

/// ASID allocator.
///
/// This structure manages the allocation and deallocation of ASIDs.
/// ASIDs are used to avoid TLB flushes when switching between processes.
struct AsidAllocator {
    /// The bitmap of allocated ASIDs.
    /// Each bit represents an ASID, where 1 means allocated and 0 means free.
    /// ASIDs start from ASID_MIN.
    bitmap: [u64; (ASID_CAP as usize - ASID_MIN as usize).div_ceil(64)],

    /// The next ASID to try to allocate.
    next: u16,
}

impl AsidAllocator {
    /// Creates a new ASID allocator.
    const fn new() -> Self {
        Self {
            bitmap: [0; (ASID_CAP as usize - ASID_MIN as usize).div_ceil(64)],
            next: ASID_MIN,
        }
    }

    /// Allocates a new ASID.
    ///
    /// Returns the allocated ASID, or `ASID_FLUSH_REQUIRED` if no ASIDs are available.
    fn allocate(&mut self) -> u16 {
        // Try to find a free ASID starting from `next`
        let start = self.next as usize - ASID_MIN as usize;

        // First search from next to end
        for i in start / 64..self.bitmap.len() {
            let word = self.bitmap[i];
            if word != u64::MAX {
                // Found a word with at least one free bit
                let bit = word.trailing_ones() as usize;
                if bit < 64 {
                    let asid = ASID_MIN as usize + i * 64 + bit;
                    if asid <= ASID_CAP as usize {
                        self.bitmap[i] |= 1 << bit;
                        self.next = (asid + 1) as u16;
                        if self.next > ASID_CAP {
                            self.next = ASID_MIN;
                        }
                        return asid as u16;
                    }
                }
            }
        }

        // Then search from beginning to next
        for i in 0..start / 64 {
            let word = self.bitmap[i];
            if word != u64::MAX {
                // Found a word with at least one free bit
                let bit = word.trailing_ones() as usize;
                if bit < 64 {
                    let asid = ASID_MIN as usize + i * 64 + bit;
                    self.bitmap[i] |= 1 << bit;
                    self.next = (asid + 1) as u16;
                    return asid as u16;
                }
            }
        }

        // No ASIDs available
        ASID_FLUSH_REQUIRED
    }

    /// Deallocates an ASID.
    fn deallocate(&mut self, asid: u16) {
        // Don't deallocate the special ASID
        if asid == ASID_FLUSH_REQUIRED {
            return;
        }

        assert!((ASID_MIN..ASID_CAP).contains(&asid), "ASID out of range");

        let index = (asid as usize - ASID_MIN as usize) / 64;
        let bit = (asid as usize - ASID_MIN as usize) % 64;

        // Deallocate the ASID
        self.bitmap[index] &= !(1 << bit);
    }
}

/// Allocates a new ASID.
///
/// Returns the allocated ASID, or `ASID_FLUSH_REQUIRED` if no ASIDs are available.
pub fn allocate() -> u16 {
    let bitmap_asid = ASID_ALLOCATOR.lock().allocate();
    if bitmap_asid != ASID_FLUSH_REQUIRED {
        let mut asid_map = ASID_MAP.lock();
        let generation = current_generation();
        asid_map.insert(bitmap_asid, generation);
        return bitmap_asid;
    }

    // If bitmap allocation failed, try BTreeMap
    let mut asid_map = ASID_MAP.lock();
    let generation = current_generation();

    // Try to find a free ASID
    if let Some(asid) = find_free_asid(&mut asid_map, generation) {
        return asid;
    }

    // If no free ASID found, increment generation and reset bitmap
    increment_generation();

    // Reset bitmap allocator
    *ASID_ALLOCATOR.lock() = AsidAllocator::new();

    // Try again
    let new_generation = current_generation();
    let bitmap_asid = ASID_ALLOCATOR.lock().allocate();
    if bitmap_asid != ASID_FLUSH_REQUIRED {
        let mut asid_map = ASID_MAP.lock();
        asid_map.insert(bitmap_asid, new_generation);
        return bitmap_asid;
    }

    let mut asid_map = ASID_MAP.lock();
    if let Some(asid) = find_free_asid(&mut asid_map, new_generation) {
        return asid;
    }

    // If still no ASID available, return ASID_FLUSH_REQUIRED
    ASID_FLUSH_REQUIRED
}

/// Finds a free ASID in the range of ASID_MIN to ASID_CAP.
///
/// Returns the found ASID if it is free, otherwise returns `None`.
fn find_free_asid(
    asid_map: &mut impl core::ops::DerefMut<Target = BTreeMap<u16, u16>>,
    generation: u16,
) -> Option<u16> {
    // Search for a free ASID in the range of ASID_MIN to ASID_CAP
    for asid in ASID_MIN..=ASID_CAP {
        if let Entry::Vacant(e) = asid_map.entry(asid) {
            log::debug!("[ASID] Found free ASID: {}", asid);
            e.insert(generation);
            return Some(asid);
        }
    }
    None
}

/// Deallocates an ASID.
pub fn deallocate(asid: u16) {
    if asid == ASID_FLUSH_REQUIRED {
        return;
    }

    let mut asid_map = ASID_MAP.lock();

    // Remove from map first
    asid_map.remove(&asid);

    // Only deallocate from bitmap if it's in the valid range for the bitmap
    if (ASID_MIN..ASID_CAP).contains(&asid) {
        ASID_ALLOCATOR.lock().deallocate(asid);
    }
}

/// Gets the current ASID generation.
pub fn current_generation() -> u16 {
    ASID_GENERATION.load(Ordering::Relaxed)
}

/// Increments the ASID generation.
///
/// This is called when we run out of ASIDs and need to flush all TLBs.
pub fn increment_generation() {
    let next_generation = ASID_GENERATION.load(Ordering::Acquire).wrapping_add(1);

    // Clear the ASID map
    let mut asid_map = ASID_MAP.lock();
    asid_map.clear();

    // Update the generation
    ASID_GENERATION.store(next_generation, Ordering::Release);
}
