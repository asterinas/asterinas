// SPDX-License-Identifier: MPL-2.0

//! A global allocator implementation of many slab caches.

use core::{
    alloc::{AllocError, Layout},
    cell::RefCell,
};

use ostd::{
    cpu_local,
    mm::{
        heap::{GlobalHeapAllocator, HeapSlot, SlabSlotList, SlotInfo},
        PAGE_SIZE,
    },
    sync::{LocalIrqDisabled, SpinLock},
    trap,
};

use crate::slab_cache::SlabCache;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub(crate) enum CommonSizeClass {
    Bytes8 = 8,
    Bytes16 = 16,
    Bytes32 = 32,
    Bytes64 = 64,
    Bytes128 = 128,
    Bytes256 = 256,
    Bytes512 = 512,
    Bytes1024 = 1024,
    Bytes2048 = 2048,
}

impl CommonSizeClass {
    pub(crate) const fn from_layout(layout: Layout) -> Option<Self> {
        let size_class = match layout.size() {
            0..=8 => CommonSizeClass::Bytes8,
            9..=16 => CommonSizeClass::Bytes16,
            17..=32 => CommonSizeClass::Bytes32,
            33..=64 => CommonSizeClass::Bytes64,
            65..=128 => CommonSizeClass::Bytes128,
            129..=256 => CommonSizeClass::Bytes256,
            257..=512 => CommonSizeClass::Bytes512,
            513..=1024 => CommonSizeClass::Bytes1024,
            1025..=2048 => CommonSizeClass::Bytes2048,
            _ => return None,
        };
        // Alignment must be non-zero and power-of-two.
        let align_class = match layout.align() {
            1 | 2 | 4 | 8 => CommonSizeClass::Bytes8,
            16 => CommonSizeClass::Bytes16,
            32 => CommonSizeClass::Bytes32,
            64 => CommonSizeClass::Bytes64,
            128 => CommonSizeClass::Bytes128,
            256 => CommonSizeClass::Bytes256,
            512 => CommonSizeClass::Bytes512,
            1024 => CommonSizeClass::Bytes1024,
            2048 => CommonSizeClass::Bytes2048,
            _ => return None,
        };
        Some(if (size_class as usize) < (align_class as usize) {
            align_class
        } else {
            size_class
        })
    }

    pub(crate) const fn from_size(size: usize) -> Option<Self> {
        match size {
            8 => Some(CommonSizeClass::Bytes8),
            16 => Some(CommonSizeClass::Bytes16),
            32 => Some(CommonSizeClass::Bytes32),
            64 => Some(CommonSizeClass::Bytes64),
            128 => Some(CommonSizeClass::Bytes128),
            256 => Some(CommonSizeClass::Bytes256),
            512 => Some(CommonSizeClass::Bytes512),
            1024 => Some(CommonSizeClass::Bytes1024),
            2048 => Some(CommonSizeClass::Bytes2048),
            _ => None,
        }
    }
}

/// Get the type of the slot from the layout.
///
/// It should be used to define [`ostd::global_heap_allocator_slot_map`].
pub const fn type_from_layout(layout: Layout) -> Option<SlotInfo> {
    if let Some(class) = CommonSizeClass::from_layout(layout) {
        return Some(SlotInfo::SlabSlot(class as usize));
    }
    if layout.size() > PAGE_SIZE / 2 && layout.align() <= PAGE_SIZE {
        return Some(SlotInfo::LargeSlot(
            layout.size().div_ceil(PAGE_SIZE) * PAGE_SIZE,
        ));
    }
    None
}

struct Heap {
    slab8: SlabCache<8>,
    slab16: SlabCache<16>,
    slab32: SlabCache<32>,
    slab64: SlabCache<64>,
    slab128: SlabCache<128>,
    slab256: SlabCache<256>,
    slab512: SlabCache<512>,
    slab1024: SlabCache<1024>,
    slab2048: SlabCache<2048>,
}

impl Heap {
    const fn new() -> Self {
        Self {
            slab8: SlabCache::new(),
            slab16: SlabCache::new(),
            slab32: SlabCache::new(),
            slab64: SlabCache::new(),
            slab128: SlabCache::new(),
            slab256: SlabCache::new(),
            slab512: SlabCache::new(),
            slab1024: SlabCache::new(),
            slab2048: SlabCache::new(),
        }
    }

    fn alloc(&mut self, class: CommonSizeClass) -> Result<HeapSlot, AllocError> {
        match class {
            CommonSizeClass::Bytes8 => self.slab8.alloc(),
            CommonSizeClass::Bytes16 => self.slab16.alloc(),
            CommonSizeClass::Bytes32 => self.slab32.alloc(),
            CommonSizeClass::Bytes64 => self.slab64.alloc(),
            CommonSizeClass::Bytes128 => self.slab128.alloc(),
            CommonSizeClass::Bytes256 => self.slab256.alloc(),
            CommonSizeClass::Bytes512 => self.slab512.alloc(),
            CommonSizeClass::Bytes1024 => self.slab1024.alloc(),
            CommonSizeClass::Bytes2048 => self.slab2048.alloc(),
        }
    }

    fn dealloc(&mut self, slot: HeapSlot, class: CommonSizeClass) -> Result<(), AllocError> {
        match class {
            CommonSizeClass::Bytes8 => self.slab8.dealloc(slot),
            CommonSizeClass::Bytes16 => self.slab16.dealloc(slot),
            CommonSizeClass::Bytes32 => self.slab32.dealloc(slot),
            CommonSizeClass::Bytes64 => self.slab64.dealloc(slot),
            CommonSizeClass::Bytes128 => self.slab128.dealloc(slot),
            CommonSizeClass::Bytes256 => self.slab256.dealloc(slot),
            CommonSizeClass::Bytes512 => self.slab512.dealloc(slot),
            CommonSizeClass::Bytes1024 => self.slab1024.dealloc(slot),
            CommonSizeClass::Bytes2048 => self.slab2048.dealloc(slot),
        }
    }
}

static GLOBAL_POOL: SpinLock<Heap, LocalIrqDisabled> = SpinLock::new(Heap::new());

/// The maximum size in bytes of the object cache of each slot size class.
const OBJ_CACHE_MAX_SIZE: usize = 8 * PAGE_SIZE;
/// The expected size in bytes of the object cache of each slot size class.
///
/// If the cache exceeds the maximum size or is empty, it will be adjusted to
/// this size.
const OBJ_CACHE_EXPECTED_SIZE: usize = 2 * PAGE_SIZE;

struct ObjectCache<const SLOT_SIZE: usize> {
    list: SlabSlotList<SLOT_SIZE>,
    list_size: usize,
}

impl<const SLOT_SIZE: usize> ObjectCache<SLOT_SIZE> {
    const fn new() -> Self {
        Self {
            list: SlabSlotList::new(),
            list_size: 0,
        }
    }

    fn alloc(&mut self) -> Result<HeapSlot, AllocError> {
        if let Some(slot) = self.list.pop() {
            self.list_size -= SLOT_SIZE;
            return Ok(slot);
        }

        let size_class = CommonSizeClass::from_size(SLOT_SIZE).unwrap();
        let mut global_pool = GLOBAL_POOL.lock();
        for _ in 0..OBJ_CACHE_EXPECTED_SIZE / SLOT_SIZE {
            if let Ok(slot) = global_pool.alloc(size_class) {
                self.list.push(slot);
                self.list_size += SLOT_SIZE;
            } else {
                break;
            }
        }

        if let Ok(new_slot) = global_pool.alloc(size_class) {
            Ok(new_slot)
        } else if let Some(popped) = self.list.pop() {
            self.list_size -= SLOT_SIZE;
            Ok(popped)
        } else {
            Err(AllocError)
        }
    }

    fn dealloc(&mut self, slot: HeapSlot, class: CommonSizeClass) -> Result<(), AllocError> {
        if self.list_size + SLOT_SIZE < OBJ_CACHE_MAX_SIZE {
            self.list.push(slot);
            self.list_size += SLOT_SIZE;
            return Ok(());
        }

        let mut global_pool = GLOBAL_POOL.lock();
        global_pool.dealloc(slot, class)?;
        for _ in 0..(self.list_size - OBJ_CACHE_EXPECTED_SIZE) / SLOT_SIZE {
            let slot = self.list.pop().expect("The cache size should be ample");
            global_pool.dealloc(slot, class)?;
            self.list_size -= SLOT_SIZE;
        }

        Ok(())
    }
}

struct LocalCache {
    cache8: ObjectCache<8>,
    cache16: ObjectCache<16>,
    cache32: ObjectCache<32>,
    cache64: ObjectCache<64>,
    cache128: ObjectCache<128>,
    cache256: ObjectCache<256>,
    cache512: ObjectCache<512>,
    cache1024: ObjectCache<1024>,
    cache2048: ObjectCache<2048>,
}

impl LocalCache {
    const fn new() -> Self {
        Self {
            cache8: ObjectCache::new(),
            cache16: ObjectCache::new(),
            cache32: ObjectCache::new(),
            cache64: ObjectCache::new(),
            cache128: ObjectCache::new(),
            cache256: ObjectCache::new(),
            cache512: ObjectCache::new(),
            cache1024: ObjectCache::new(),
            cache2048: ObjectCache::new(),
        }
    }

    fn alloc(&mut self, class: CommonSizeClass) -> Result<HeapSlot, AllocError> {
        match class {
            CommonSizeClass::Bytes8 => self.cache8.alloc(),
            CommonSizeClass::Bytes16 => self.cache16.alloc(),
            CommonSizeClass::Bytes32 => self.cache32.alloc(),
            CommonSizeClass::Bytes64 => self.cache64.alloc(),
            CommonSizeClass::Bytes128 => self.cache128.alloc(),
            CommonSizeClass::Bytes256 => self.cache256.alloc(),
            CommonSizeClass::Bytes512 => self.cache512.alloc(),
            CommonSizeClass::Bytes1024 => self.cache1024.alloc(),
            CommonSizeClass::Bytes2048 => self.cache2048.alloc(),
        }
    }

    fn dealloc(&mut self, slot: HeapSlot, class: CommonSizeClass) -> Result<(), AllocError> {
        match class {
            CommonSizeClass::Bytes8 => self.cache8.dealloc(slot, class),
            CommonSizeClass::Bytes16 => self.cache16.dealloc(slot, class),
            CommonSizeClass::Bytes32 => self.cache32.dealloc(slot, class),
            CommonSizeClass::Bytes64 => self.cache64.dealloc(slot, class),
            CommonSizeClass::Bytes128 => self.cache128.dealloc(slot, class),
            CommonSizeClass::Bytes256 => self.cache256.dealloc(slot, class),
            CommonSizeClass::Bytes512 => self.cache512.dealloc(slot, class),
            CommonSizeClass::Bytes1024 => self.cache1024.dealloc(slot, class),
            CommonSizeClass::Bytes2048 => self.cache2048.dealloc(slot, class),
        }
    }
}

cpu_local! {
    static LOCAL_POOL: RefCell<LocalCache> = RefCell::new(LocalCache::new());
}

/// The global heap allocator provided by OSDK.
///
/// It is a singleton that provides heap allocation for the kernel. If
/// multiple instances of this struct are created, all the member functions
/// will eventually access the same allocator.
pub struct HeapAllocator;

impl GlobalHeapAllocator for HeapAllocator {
    fn alloc(&self, layout: Layout) -> Result<HeapSlot, AllocError> {
        let Some(class) = CommonSizeClass::from_layout(layout) else {
            return HeapSlot::alloc_large(layout.size().div_ceil(PAGE_SIZE) * PAGE_SIZE);
        };

        let irq_guard = trap::irq::disable_local();
        let this_cache = LOCAL_POOL.get_with(&irq_guard);
        let mut local_cache = this_cache.borrow_mut();

        local_cache.alloc(class)
    }

    fn dealloc(&self, slot: HeapSlot) -> Result<(), AllocError> {
        let Some(class) = CommonSizeClass::from_size(slot.size()) else {
            slot.dealloc_large();
            return Ok(());
        };

        let irq_guard = trap::irq::disable_local();
        let this_cache = LOCAL_POOL.get_with(&irq_guard);
        let mut local_cache = this_cache.borrow_mut();

        local_cache.dealloc(slot, class)
    }
}
