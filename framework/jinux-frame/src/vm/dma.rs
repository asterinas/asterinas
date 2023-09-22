use crate::arch::iommu::{has_iommu, iova};

use super::Paddr;

pub type Daddr = usize;

pub fn has_tdx() -> bool {
    // FIXME: Support TDX
    false
}

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
