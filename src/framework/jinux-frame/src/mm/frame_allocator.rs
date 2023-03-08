use alloc::vec::Vec;
use buddy_system_allocator::FrameAllocator;
use limine::{LimineMemmapEntry, LimineMemoryMapEntryType};
use log::info;
use spin::{Mutex, Once};

use crate::{config::PAGE_SIZE, vm::Paddr};

use super::address::PhysAddr;

static  FRAME_ALLOCATOR: Once<Mutex<FrameAllocator>> = Once::new();


#[derive(Debug, Clone)]
// #[repr(transparent)]
pub struct PhysFrame {
    frame_index: usize,
    need_dealloc: bool,
}

impl PhysFrame {
    pub const fn start_pa(&self) -> PhysAddr {
        PhysAddr(self.frame_index * PAGE_SIZE)
    }

    pub const fn end_pa(&self) -> PhysAddr {
        PhysAddr((self.frame_index + 1) * PAGE_SIZE)
    }

    pub fn alloc() -> Option<Self> {
        FRAME_ALLOCATOR.get().unwrap().lock().alloc(1).map(|pa| Self {
            frame_index: pa,
            need_dealloc: true,
        })
    }

    pub fn alloc_continuous_range(frame_count: usize) -> Option<Vec<Self>> {
        FRAME_ALLOCATOR.get().unwrap().lock().alloc(frame_count).map(|start| {
            let mut vector = Vec::new();
            for i in 0..frame_count {
                vector.push(Self {
                    frame_index: start + i,
                    need_dealloc: true,
                })
            }
            vector
        })
    }

    pub fn alloc_with_paddr(paddr: Paddr) -> Option<Self> {
        // FIXME: need to check whether the physical address is invalid or not
        Some(Self {
            frame_index: paddr / PAGE_SIZE,
            need_dealloc: false,
        })
    }

    pub fn alloc_zero() -> Option<Self> {
        let mut f = Self::alloc()?;
        f.zero();
        Some(f)
    }

    pub fn zero(&mut self) {
        unsafe { core::ptr::write_bytes(self.start_pa().kvaddr().as_ptr(), 0, PAGE_SIZE) }
    }

    pub fn as_slice(&self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.start_pa().kvaddr().as_ptr(), PAGE_SIZE) }
    }
}

impl Drop for PhysFrame {
    fn drop(&mut self) {
        if self.need_dealloc {
            FRAME_ALLOCATOR.get().unwrap().lock().dealloc(self.frame_index, 1);
        }
    }
}

pub(crate) fn init(regions: &Vec<&LimineMemmapEntry>) {
    let mut allocator = FrameAllocator::<32>::new();
    for region in regions.iter() {
        if region.typ == LimineMemoryMapEntryType::Usable {
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
    FRAME_ALLOCATOR.call_once(||Mutex::new(allocator));
}