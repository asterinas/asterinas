mod dma_coherent;
mod dma_stream;

use alloc::collections::BTreeSet;
use spin::Once;

use crate::{arch::iommu::has_iommu, config::PAGE_SIZE, sync::SpinLock};

use super::Paddr;

pub use dma_coherent::DmaCoherent;
pub use dma_stream::{DmaDirection, DmaStream};

/// If a device performs DMA to read or write system
/// memory, the addresses used by the device are device addresses.
/// Daddr can distinguish the address space used by cpu side and
/// the address space used by device side.
pub type Daddr = usize;

fn has_tdx() -> bool {
    // FIXME: Support TDX
    false
}

#[derive(PartialEq)]
pub enum DmaType {
    Direct,
    Iommu,
    Tdx,
}

#[derive(Debug)]
pub enum DmaError {
    InvalidArgs,
    AlreadyMapped,
}

pub trait HasDaddr {
    /// Get the base address of the mapping in the
    /// device address space.
    fn daddr(&self) -> Daddr;
}

/// Set of all physical addresses with dma mapping.
static DMA_MAPPING_SET: Once<SpinLock<BTreeSet<Paddr>>> = Once::new();

pub fn dma_type() -> DmaType {
    if has_iommu() {
        DmaType::Iommu
    } else if has_tdx() {
        return DmaType::Tdx;
    } else {
        return DmaType::Direct;
    }
}

pub fn init() {
    DMA_MAPPING_SET.call_once(|| SpinLock::new(BTreeSet::new()));
}

/// Check whether the physical addresses has dma mapping.
/// Fail if they have been mapped, otherwise insert them.
fn check_and_insert_dma_mapping(start_paddr: Paddr, num_pages: usize) -> bool {
    let mut mapping_set = DMA_MAPPING_SET.get().unwrap().lock_irq_disabled();
    for i in 0..num_pages {
        let paddr = start_paddr + (i * PAGE_SIZE);
        if mapping_set.contains(&paddr) {
            return false;
        }
    }
    for i in 0..num_pages {
        let paddr = start_paddr + (i * PAGE_SIZE);
        mapping_set.insert(paddr);
    }
    true
}

/// Remove a physical address from the dma mapping set.
fn remove_dma_mapping(start_paddr: Paddr, num_pages: usize) {
    let mut mapping_set = DMA_MAPPING_SET.get().unwrap().lock_irq_disabled();
    for i in 0..num_pages {
        let paddr = start_paddr + (i * PAGE_SIZE);
        mapping_set.remove(&paddr);
    }
}
