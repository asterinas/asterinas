// SPDX-License-Identifier: MPL-2.0

//! NVMe PRP (Physical Region Page) setup for contiguous DMA buffers.

use alloc::vec::Vec;
use core::num::NonZeroUsize;

use ostd::mm::{HasDaddr, PAGE_SIZE, VmIo, dma::DmaCoherent};

use crate::device::NvmeDeviceError;

/// Number of 64-bit PRP entries that fit in one PRP list page.
const PRP_ENTRIES_PER_PAGE: usize = PAGE_SIZE / size_of::<u64>();

const PAGE_SIZE_U64: u64 = PAGE_SIZE as u64;

pub(crate) struct PrpPointers {
    prp1: u64,
    prp2: u64,
    #[cfg_attr(not(ktest), expect(dead_code))]
    list_pages: Vec<DmaCoherent>,
}

impl PrpPointers {
    /// Builds PRP pointers for a physically contiguous `[dma_addr, dma_addr + length)` range.
    pub(crate) fn build_prp(dma_addr: u64, length: NonZeroUsize) -> Result<Self, NvmeDeviceError> {
        let prp1 = dma_addr;
        let remaining = remaining_bytes_after_first_page(dma_addr, length.get());
        if remaining == 0 {
            return Ok(Self::without_list(prp1, 0));
        }

        let second_page = next_page_addr(dma_addr);
        if remaining <= PAGE_SIZE_U64 {
            return Ok(Self::without_list(prp1, second_page));
        }

        Self::with_list(prp1, second_page, remaining)
    }

    pub(crate) fn prp1(&self) -> u64 {
        self.prp1
    }

    pub(crate) fn prp2(&self) -> u64 {
        self.prp2
    }

    fn without_list(prp1: u64, prp2: u64) -> Self {
        Self {
            prp1,
            prp2,
            list_pages: Vec::new(),
        }
    }

    fn with_list(prp1: u64, mut addr: u64, mut remaining: u64) -> Result<Self, NvmeDeviceError> {
        let mut list_pages = Vec::with_capacity(
            (remaining as usize).div_ceil(PAGE_SIZE * (PRP_ENTRIES_PER_PAGE - 1)),
        );

        let first_list_page =
            DmaCoherent::alloc(1, true).map_err(|_| NvmeDeviceError::DmaAllocationFailed)?;
        let prp2 = first_list_page.daddr() as u64;
        list_pages.push(first_list_page);

        let mut cur_list_page = &list_pages[0];
        let mut cur_list_index = 0usize;

        while remaining > 0 {
            // The last entry of a non-last PRP list page must point to the next list
            // page; the last entry of the last list page may point to a data page.
            if cur_list_index == PRP_ENTRIES_PER_PAGE - 1 && remaining > PAGE_SIZE_U64 {
                let new_list_page = DmaCoherent::alloc(1, true)
                    .map_err(|_| NvmeDeviceError::DmaAllocationFailed)?;
                write_prp_entry(cur_list_page, cur_list_index, new_list_page.daddr() as u64);
                list_pages.push(new_list_page);

                cur_list_page = list_pages.last().unwrap();
                cur_list_index = 0;
            }

            write_prp_entry(cur_list_page, cur_list_index, addr);
            cur_list_index += 1;

            if remaining <= PAGE_SIZE_U64 {
                break;
            }
            addr += PAGE_SIZE_U64;
            remaining -= PAGE_SIZE_U64;
        }

        Ok(Self {
            prp1,
            prp2,
            list_pages,
        })
    }
}

/// Returns how many bytes of `length` lie after the page containing `dma_addr`.
fn remaining_bytes_after_first_page(dma_addr: u64, length: usize) -> u64 {
    let offset = dma_addr & (PAGE_SIZE_U64 - 1);
    (length as u64).saturating_sub(PAGE_SIZE_U64 - offset)
}

/// Returns the physical address of the page immediately after the one containing `dma_addr`.
fn next_page_addr(dma_addr: u64) -> u64 {
    let page_mask = PAGE_SIZE_U64 - 1;
    (dma_addr & !page_mask) + PAGE_SIZE_U64
}

#[cfg(ktest)]
fn read_prp_entry(list: &DmaCoherent, index: usize) -> u64 {
    debug_assert!(index < PRP_ENTRIES_PER_PAGE);
    u64::from_le_bytes(list.read_val::<[u8; 8]>(index * size_of::<u64>()).unwrap())
}

fn write_prp_entry(list: &DmaCoherent, index: usize, addr: u64) {
    debug_assert!(index < PRP_ENTRIES_PER_PAGE);
    list.write_val(index * size_of::<u64>(), &addr.to_le_bytes())
        .unwrap();
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn single_page_transfer_uses_prp1_only() {
        let dma = DmaCoherent::alloc(1, true).unwrap();
        let addr = dma.daddr() as u64;
        let prp = PrpPointers::build_prp(addr, NonZeroUsize::new(PAGE_SIZE / 2).unwrap()).unwrap();
        assert_eq!(prp.prp1(), addr);
        assert_eq!(prp.prp2(), 0);
        assert!(prp.list_pages.is_empty());
    }

    #[ktest]
    fn two_page_transfer_uses_prp2() {
        let dma = DmaCoherent::alloc(2, true).unwrap();
        let addr = dma.daddr() as u64;
        let prp = PrpPointers::build_prp(addr, NonZeroUsize::new(PAGE_SIZE * 2).unwrap()).unwrap();
        assert_eq!(prp.prp1(), addr);
        assert_eq!(prp.prp2(), addr + PAGE_SIZE_U64);
        assert!(prp.list_pages.is_empty());
    }

    #[ktest]
    fn four_page_transfer_fills_prp_list() {
        let dma = DmaCoherent::alloc(4, true).unwrap();
        let addr = dma.daddr() as u64;
        let prp = PrpPointers::build_prp(addr, NonZeroUsize::new(PAGE_SIZE * 4).unwrap()).unwrap();

        assert_eq!(prp.prp1(), addr);
        assert_eq!(prp.prp2(), prp.list_pages[0].daddr() as u64);
        assert_eq!(prp.list_pages.len(), 1);

        for page_index in 1..4 {
            assert_eq!(
                read_prp_entry(&prp.list_pages[0], page_index - 1),
                addr + page_index as u64 * PAGE_SIZE_U64
            );
        }
    }

    #[ktest]
    fn six_hundred_page_transfer_chains_prp_list() {
        const NR_PAGES: u64 = 600;

        let dma = DmaCoherent::alloc(1, true).unwrap();
        let addr = dma.daddr() as u64;
        let prp = PrpPointers::build_prp(
            addr,
            NonZeroUsize::new((NR_PAGES as usize) * PAGE_SIZE).unwrap(),
        )
        .unwrap();

        assert_eq!(prp.prp1(), addr);
        assert_eq!(prp.prp2(), prp.list_pages[0].daddr() as u64);
        assert_eq!(prp.list_pages.len(), 2);

        let first_list = &prp.list_pages[0];
        let second_list = &prp.list_pages[1];

        for page_index in 1..512 {
            assert_eq!(
                read_prp_entry(first_list, (page_index - 1) as usize),
                addr + page_index * PAGE_SIZE_U64
            );
        }
        assert_eq!(
            read_prp_entry(first_list, PRP_ENTRIES_PER_PAGE - 1),
            second_list.daddr() as u64
        );
        for page_index in 512..NR_PAGES {
            assert_eq!(
                read_prp_entry(second_list, (page_index - 512) as usize),
                addr + page_index * PAGE_SIZE_U64
            );
        }
    }
}
