use crate::bus::pci::PciDeviceLocation;
use crate::config::PAGE_SIZE;
use crate::sync::Mutex;
use crate::vm::dma::Daddr;
use crate::vm::Paddr;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use buddy_system_allocator::FrameAllocator;
use spin::Once;

static IOVA_ALLOCATOR: Once<Mutex<IovaAllocator>> = Once::new();

struct IovaAllocator {
    // map the device id to its iova allocator
    allocators: BTreeMap<PciDeviceLocation, FrameAllocator>,
    // record which device address the physical address is mapped to
    rmap: BTreeMap<Paddr, Daddr>,
}

pub(crate) fn init() {
    let mut iova_allocator = IovaAllocator {
        allocators: BTreeMap::new(),
        rmap: BTreeMap::new(),
    };

    for table in PciDeviceLocation::all() {
        let mut allocator = FrameAllocator::<32>::new();
        allocator.add_frame(0, (1 << 32) / PAGE_SIZE);
        iova_allocator.allocators.insert(table, allocator);
    }

    IOVA_ALLOCATOR.call_once(|| Mutex::new(iova_allocator));
}

pub(crate) fn alloc_iova(device_id: PciDeviceLocation) -> Option<Daddr> {
    IOVA_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .allocators
        .get_mut(&device_id)
        .unwrap()
        .alloc(1)
        .map(|daddr| daddr * PAGE_SIZE)
}

pub(crate) fn alloc_iova_continuous(
    device_id: PciDeviceLocation,
    count: usize,
) -> Option<Vec<Daddr>> {
    IOVA_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .allocators
        .get_mut(&device_id)
        .unwrap()
        .alloc(count)
        .map(|start_daddr| {
            let mut vector = Vec::new();
            for i in 0..count {
                vector.push((start_daddr + i) * PAGE_SIZE);
            }
            vector
        })
}

pub(crate) fn dealloc_iova(device_id: PciDeviceLocation, daddr: Daddr) {
    IOVA_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .allocators
        .get_mut(&device_id)
        .unwrap()
        .dealloc(daddr / PAGE_SIZE, 1);
}

pub(crate) fn rmap_paddr(paddr: Paddr, daddr: Daddr) {
    IOVA_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .rmap
        .insert(paddr / PAGE_SIZE, daddr / PAGE_SIZE);
}

pub(crate) fn remove_paddr(paddr: Paddr) {
    IOVA_ALLOCATOR.get().unwrap().lock().rmap.remove(&paddr);
}

pub(crate) fn paddr_to_daddr(paddr: Paddr) -> Option<Daddr> {
    IOVA_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .rmap
        .get(&(paddr / PAGE_SIZE))
        .map(|daddr| (*daddr) * PAGE_SIZE + (paddr & (PAGE_SIZE - 1)))
}
