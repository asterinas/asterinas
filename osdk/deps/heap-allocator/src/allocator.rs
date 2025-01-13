// SPDX-License-Identifier: MPL-2.0

//! A global allocator implementation of many slab caches.

use core::{
    alloc::{AllocError, Layout},
    cell::RefCell,
};

use ostd::{
    cpu_local,
    mm::{
        heap::{slot::HeapSlot, slot_list::HeapSlotList, GlobalHeapAllocator},
        PAGE_SIZE,
    },
    sync::{LocalIrqDisabled, SpinLock},
    trap,
};

use crate::slab_cache::SlabCache;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
enum CommonSizeClasses {
    Bytes8 = 8,
    Bytes16 = 16,
    Bytes32 = 32,
    Bytes64 = 64,
    Bytes128 = 128,
    Bytes256 = 256,
    Bytes512 = 512,
    Bytes1024 = 1024,
    Bytes2048 = 2048,
    Bytes4096 = 4096,
}

impl CommonSizeClasses {
    fn from_slot_size(slot_size: usize) -> Self {
        match slot_size {
            0..=8 => CommonSizeClasses::Bytes8,
            9..=16 => CommonSizeClasses::Bytes16,
            17..=32 => CommonSizeClasses::Bytes32,
            33..=64 => CommonSizeClasses::Bytes64,
            65..=128 => CommonSizeClasses::Bytes128,
            129..=256 => CommonSizeClasses::Bytes256,
            257..=512 => CommonSizeClasses::Bytes512,
            513..=1024 => CommonSizeClasses::Bytes1024,
            1025..=2048 => CommonSizeClasses::Bytes2048,
            2049..=4096 => CommonSizeClasses::Bytes4096,
            _ => panic!("Invalid slot size"),
        }
    }

    fn align_to(self, align: usize) -> Self {
        if align <= self as usize {
            return self;
        }

        Self::from_slot_size(align)
    }
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
    slab4096: SlabCache<4096>,
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
            slab4096: SlabCache::new(),
        }
    }

    fn alloc(&mut self, class: CommonSizeClasses) -> Result<HeapSlot, AllocError> {
        match class {
            CommonSizeClasses::Bytes8 => self.slab8.alloc(),
            CommonSizeClasses::Bytes16 => self.slab16.alloc(),
            CommonSizeClasses::Bytes32 => self.slab32.alloc(),
            CommonSizeClasses::Bytes64 => self.slab64.alloc(),
            CommonSizeClasses::Bytes128 => self.slab128.alloc(),
            CommonSizeClasses::Bytes256 => self.slab256.alloc(),
            CommonSizeClasses::Bytes512 => self.slab512.alloc(),
            CommonSizeClasses::Bytes1024 => self.slab1024.alloc(),
            CommonSizeClasses::Bytes2048 => self.slab2048.alloc(),
            CommonSizeClasses::Bytes4096 => self.slab4096.alloc(),
        }
    }

    fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        match CommonSizeClasses::from_slot_size(slot.size()) {
            CommonSizeClasses::Bytes8 => self.slab8.dealloc(slot),
            CommonSizeClasses::Bytes16 => self.slab16.dealloc(slot),
            CommonSizeClasses::Bytes32 => self.slab32.dealloc(slot),
            CommonSizeClasses::Bytes64 => self.slab64.dealloc(slot),
            CommonSizeClasses::Bytes128 => self.slab128.dealloc(slot),
            CommonSizeClasses::Bytes256 => self.slab256.dealloc(slot),
            CommonSizeClasses::Bytes512 => self.slab512.dealloc(slot),
            CommonSizeClasses::Bytes1024 => self.slab1024.dealloc(slot),
            CommonSizeClasses::Bytes2048 => self.slab2048.dealloc(slot),
            CommonSizeClasses::Bytes4096 => self.slab4096.dealloc(slot),
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
    list: HeapSlotList<SLOT_SIZE>,
    list_size: usize,
}

impl<const SLOT_SIZE: usize> ObjectCache<SLOT_SIZE> {
    const fn new() -> Self {
        Self {
            list: HeapSlotList::new(),
            list_size: 0,
        }
    }

    fn alloc(&mut self) -> Result<HeapSlot, AllocError> {
        if let Some(slot) = self.list.pop() {
            self.list_size -= SLOT_SIZE;
            return Ok(slot);
        }

        let size_class = CommonSizeClasses::from_slot_size(SLOT_SIZE);
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

    fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        if self.list_size + SLOT_SIZE < OBJ_CACHE_MAX_SIZE {
            self.list.push(slot);
            self.list_size += SLOT_SIZE;
            return Ok(());
        }

        let mut global_pool = GLOBAL_POOL.lock();
        global_pool.dealloc(slot)?;
        for _ in 0..(self.list_size - OBJ_CACHE_EXPECTED_SIZE) / SLOT_SIZE {
            let slot = self.list.pop().expect("The cache size should be ample");
            global_pool.dealloc(slot)?;
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
    cache4096: ObjectCache<4096>,
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
            cache4096: ObjectCache::new(),
        }
    }

    fn alloc(&mut self, class: CommonSizeClasses) -> Result<HeapSlot, AllocError> {
        match class {
            CommonSizeClasses::Bytes8 => self.cache8.alloc(),
            CommonSizeClasses::Bytes16 => self.cache16.alloc(),
            CommonSizeClasses::Bytes32 => self.cache32.alloc(),
            CommonSizeClasses::Bytes64 => self.cache64.alloc(),
            CommonSizeClasses::Bytes128 => self.cache128.alloc(),
            CommonSizeClasses::Bytes256 => self.cache256.alloc(),
            CommonSizeClasses::Bytes512 => self.cache512.alloc(),
            CommonSizeClasses::Bytes1024 => self.cache1024.alloc(),
            CommonSizeClasses::Bytes2048 => self.cache2048.alloc(),
            CommonSizeClasses::Bytes4096 => self.cache4096.alloc(),
        }
    }

    fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        match CommonSizeClasses::from_slot_size(slot.size()) {
            CommonSizeClasses::Bytes8 => self.cache8.dealloc(slot),
            CommonSizeClasses::Bytes16 => self.cache16.dealloc(slot),
            CommonSizeClasses::Bytes32 => self.cache32.dealloc(slot),
            CommonSizeClasses::Bytes64 => self.cache64.dealloc(slot),
            CommonSizeClasses::Bytes128 => self.cache128.dealloc(slot),
            CommonSizeClasses::Bytes256 => self.cache256.dealloc(slot),
            CommonSizeClasses::Bytes512 => self.cache512.dealloc(slot),
            CommonSizeClasses::Bytes1024 => self.cache1024.dealloc(slot),
            CommonSizeClasses::Bytes2048 => self.cache2048.dealloc(slot),
            CommonSizeClasses::Bytes4096 => self.cache4096.dealloc(slot),
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
        if layout.size() > 4096 {
            return HeapSlot::alloc_large(layout.size());
        }

        let irq_guard = trap::disable_local();
        let this_pool = LOCAL_POOL.get_with(&irq_guard);
        let mut local_pool = this_pool.borrow_mut();
        let class = CommonSizeClasses::from_slot_size(layout.size());
        let class = class.align_to(layout.align());

        local_pool.alloc(class)
    }

    fn dealloc(&self, slot: HeapSlot) -> Result<(), AllocError> {
        if slot.size() > 4096 {
            slot.dealloc_large();
            return Ok(());
        }

        let irq_guard = trap::disable_local();
        let this_pool = LOCAL_POOL.get_with(&irq_guard);
        let mut local_pool = this_pool.borrow_mut();

        local_pool.dealloc(slot)
    }
}
