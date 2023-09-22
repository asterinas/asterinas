mod dma_area;
mod dma_area_pool;

use core::arch::x86_64::_mm_clflush;

pub use dma_area::DmaArea;
pub use dma_area_pool::DmaAreaPool;

use crate::{
    arch::iommu::{has_iommu, iova},
    config::PAGE_SIZE,
};

use super::{Paddr, VmFrameVec};

pub type Daddr = usize;

pub fn has_tdx() -> bool {
    // FIXME: Support TDX
    false
}

#[derive(PartialEq)]
pub enum DmaType {
    Direct,
    Iommu,
    Tdx,
}

pub fn dma_type() -> DmaType {
    if has_iommu() {
        DmaType::Iommu
    } else if has_tdx() {
        return DmaType::Tdx;
    } else {
        return DmaType::Direct;
    }
}

pub fn paddr_to_daddr(pa: Paddr) -> Option<Daddr> {
    match dma_type() {
        DmaType::Direct => Some(pa as Daddr),
        DmaType::Iommu => iova::paddr_to_daddr(pa),
        DmaType::Tdx => {
            todo!()
        }
    }
}

/// Ensure that both device side and CPU side see consistent data.
/// flushing cache for both direct mapping and IOMMU mapping scenarios.
pub fn sync_frame_vec(vm_frame_vec: &VmFrameVec) {
    if dma_type() == DmaType::Tdx {
        // copy pages.
        todo!("support dma for tdx")
    } else {
        for frame in vm_frame_vec.iter() {
            for i in 0..PAGE_SIZE {
                unsafe {
                    _mm_clflush(frame.as_ptr().wrapping_add(i));
                }
            }
        }
    }
}
