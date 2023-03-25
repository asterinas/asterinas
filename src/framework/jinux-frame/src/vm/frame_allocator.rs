use core::ops::Range;

use alloc::vec::Vec;
use buddy_system_allocator::FrameAllocator;
use log::{debug, info};
use spin::{Mutex, Once};

use crate::{config::PAGE_SIZE, vm::Paddr, AlignExt};

use super::{frame::VmFrameFlags, MemoryRegions, MemoryRegionsType, VmFrame};

static FRAME_ALLOCATOR: Once<Mutex<FrameAllocator>> = Once::new();

pub fn alloc() -> Option<VmFrame> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(1)
        .map(|pa| unsafe { VmFrame::new(pa * PAGE_SIZE, VmFrameFlags::NEED_DEALLOC) })
}

pub fn alloc_continuous(frame_count: usize) -> Option<Vec<VmFrame>> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(frame_count)
        .map(|start| {
            let mut vector = Vec::new();
            unsafe {
                for i in 0..frame_count {
                    vector.push(VmFrame::new(
                        (start + i) * PAGE_SIZE,
                        VmFrameFlags::NEED_DEALLOC,
                    ))
                }
            }
            vector
        })
}

pub fn alloc_with_paddr(paddr: Paddr) -> Option<VmFrame> {
    if !is_paddr_valid(paddr..paddr + PAGE_SIZE) {
        debug!("not a valid paddr:{:x}", paddr);
        return None;
    }
    unsafe {
        Some(VmFrame::new(
            paddr.align_down(PAGE_SIZE),
            VmFrameFlags::empty(),
        ))
    }
}

/// Check if the physical address in range is valid
fn is_paddr_valid(range: Range<usize>) -> bool {
    // special area in x86
    #[cfg(feature = "x86_64")]
    if range.start >= 0xFE00_0000 && range.end <= 0xFFFF_FFFF {
        return true;
    }

    for i in super::MEMORY_REGIONS.get().unwrap().iter() {
        match i.typ {
            MemoryRegionsType::Usable => {}
            MemoryRegionsType::Reserved => {}
            MemoryRegionsType::Framebuffer => {}
            _ => {
                continue;
            }
        }
        if range.start as u64 >= i.base && (range.end as u64) <= i.base + i.len {
            return true;
        }
    }
    false
}

pub(crate) fn alloc_zero() -> Option<VmFrame> {
    let frame = alloc()?;
    frame.zero();
    Some(frame)
}

/// Dealloc a frame.
///
/// # Safety
///
/// User should ensure the index is valid
///
pub(crate) unsafe fn dealloc(index: usize) {
    FRAME_ALLOCATOR.get().unwrap().lock().dealloc(index, 1);
}

pub(crate) fn init(regions: &Vec<MemoryRegions>) {
    let mut allocator = FrameAllocator::<32>::new();
    for region in regions.iter() {
        if region.typ == MemoryRegionsType::Usable {
            assert_eq!(region.base % PAGE_SIZE as u64, 0);
            assert_eq!(region.len % PAGE_SIZE as u64, 0);
            let start = region.base as usize / PAGE_SIZE;
            let end = start + region.len as usize / PAGE_SIZE;
            allocator.add_frame(start, end);
            info!(
                "Found usable region, start:{:x}, end:{:x}",
                region.base,
                region.base + region.len
            );
        }
    }
    FRAME_ALLOCATOR.call_once(|| Mutex::new(allocator));
}
