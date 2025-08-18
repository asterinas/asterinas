// SPDX-License-Identifier: MPL-2.0

mod dma_coherent;
mod dma_stream;
#[cfg(ktest)]
mod test;

use alloc::collections::BTreeSet;

pub use dma_coherent::DmaCoherent;
pub use dma_stream::{DmaDirection, DmaStream, DmaStreamSlice};
use spin::Once;

use super::{Daddr, Paddr};
use crate::{arch::iommu::has_dma_remapping, mm::PAGE_SIZE, sync::SpinLock};

#[derive(PartialEq)]
pub enum DmaType {
    Direct,
    Iommu,
}

#[derive(Debug, PartialEq)]
pub enum DmaError {
    InvalidArgs,
    AlreadyMapped,
}

/// Set of all physical addresses with dma mapping.
static DMA_MAPPING_SET: Once<SpinLock<BTreeSet<Paddr>>> = Once::new();

pub fn dma_type() -> DmaType {
    if has_dma_remapping() {
        DmaType::Iommu
    } else {
        DmaType::Direct
    }
}

pub fn init() {
    DMA_MAPPING_SET.call_once(|| SpinLock::new(BTreeSet::new()));
}

/// Checks whether the physical addresses has dma mapping.
/// Fail if they have been mapped, otherwise insert them.
fn check_and_insert_dma_mapping(paddr: Paddr, num_pages: usize) -> bool {
    let mut mapping_set = DMA_MAPPING_SET.get().unwrap().disable_irq().lock();
    // Ensure that the addresses used later will not overflow
    paddr.checked_add(num_pages * PAGE_SIZE).unwrap();
    for i in 0..num_pages {
        let paddr = paddr + (i * PAGE_SIZE);
        if mapping_set.contains(&paddr) {
            return false;
        }
    }
    for i in 0..num_pages {
        let paddr = paddr + (i * PAGE_SIZE);
        mapping_set.insert(paddr);
    }
    true
}

/// Removes a physical address from the dma mapping set.
fn remove_dma_mapping(paddr: Paddr, num_pages: usize) {
    let mut mapping_set = DMA_MAPPING_SET.get().unwrap().disable_irq().lock();
    // Ensure that the addresses used later will not overflow
    paddr.checked_add(num_pages * PAGE_SIZE).unwrap();
    for i in 0..num_pages {
        let paddr = paddr + (i * PAGE_SIZE);
        mapping_set.remove(&paddr);
    }
}
