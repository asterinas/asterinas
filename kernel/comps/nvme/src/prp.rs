// SPDX-License-Identifier: MPL-2.0

//! NVMe PRP (Physical Region Page) setup for contiguous DMA buffers.

use alloc::vec::Vec;

use ostd::mm::{HasDaddr, PAGE_SIZE, VmIo, dma::DmaCoherent, io::util::HasVmReaderWriter};

use crate::device::NvmeDeviceError;

/// Number of 64-bit PRP entries that fit in one PRP list page.
const PRP_ENTRIES_PER_PAGE: usize = PAGE_SIZE / size_of::<u64>();

pub(crate) struct PrpPointers {
    pub prp1: u64,
    pub prp2: u64,
    list_pages: Vec<DmaCoherent>,
}

impl PrpPointers {
    /// Builds PRP pointers for a physically contiguous `[dma_addr, dma_addr + length)` range.
    pub(crate) fn build_prp(dma_addr: u64, length: usize) -> Result<Self, NvmeDeviceError> {
        if length == 0 {
            return Err(NvmeDeviceError::InvalidIoLength);
        }

        let prp1 = dma_addr;
        let remaining = remaining_bytes_after_first_page(dma_addr, length);
        if remaining == 0 {
            return Ok(Self::without_list(prp1, 0));
        }

        let second_page = next_page_addr(dma_addr);
        if remaining <= PAGE_SIZE as u64 {
            return Ok(Self::without_list(prp1, second_page));
        }

        Self::with_list(prp1, second_page, remaining)
    }

    fn without_list(prp1: u64, prp2: u64) -> Self {
        Self {
            prp1,
            prp2,
            list_pages: Vec::new(),
        }
    }

    fn with_list(prp1: u64, mut addr: u64, mut remaining: u64) -> Result<Self, NvmeDeviceError> {
        let mut list_pages = Vec::new();
        list_pages
            .push(DmaCoherent::alloc(1, true).map_err(|_| NvmeDeviceError::DmaAllocationFailed)?);
        let prp2 = list_pages[0].daddr() as u64;
        let mut active_list_index = 0usize;
        let mut list_index = 0usize;
        let page_size = PAGE_SIZE as u64;

        while remaining > 0 {
            if list_index == PRP_ENTRIES_PER_PAGE {
                active_list_index = chain_prp_list_page(&mut list_pages, active_list_index)?;
                list_index = 1;
            }

            write_prp_entry(&list_pages[active_list_index], list_index, addr)?;
            list_index += 1;
            addr = addr
                .checked_add(page_size)
                .ok_or(NvmeDeviceError::InvalidIoLength)?;
            remaining = remaining.saturating_sub(page_size);
        }

        Ok(Self {
            prp1,
            prp2,
            list_pages,
        })
    }

    pub(crate) fn list_pages(&self) -> &[DmaCoherent] {
        &self.list_pages
    }
}

/// Returns how many bytes of `length` lie after the page containing `dma_addr`.
fn remaining_bytes_after_first_page(dma_addr: u64, length: usize) -> u64 {
    let page_size = PAGE_SIZE as u64;
    let offset = dma_addr & (page_size - 1);
    (length as u64).saturating_sub(page_size - offset)
}

/// Returns the physical address of the page immediately after the one containing `dma_addr`.
fn next_page_addr(dma_addr: u64) -> u64 {
    let page_size = PAGE_SIZE as u64;
    let page_mask = page_size - 1;
    (dma_addr & !page_mask) + page_size
}

/// Links the full list page at `active_list_index` to a newly allocated page; returns the new index.
fn chain_prp_list_page(
    list_pages: &mut Vec<DmaCoherent>,
    active_list_index: usize,
) -> Result<usize, NvmeDeviceError> {
    let last_entry = read_prp_entry(&list_pages[active_list_index], PRP_ENTRIES_PER_PAGE - 1)?;
    let new_list = DmaCoherent::alloc(1, true).map_err(|_| NvmeDeviceError::DmaAllocationFailed)?;
    let chain_addr = new_list.daddr() as u64;
    write_prp_entry(&new_list, 0, last_entry)?;
    write_prp_entry(
        &list_pages[active_list_index],
        PRP_ENTRIES_PER_PAGE - 1,
        chain_addr,
    )?;
    list_pages.push(new_list);
    Ok(list_pages.len() - 1)
}

fn read_prp_entry(list: &DmaCoherent, index: usize) -> Result<u64, NvmeDeviceError> {
    debug_assert!(index < PRP_ENTRIES_PER_PAGE);
    let mut entry = [0u8; 8];
    list.read_bytes(index * size_of::<u64>(), &mut entry)
        .map_err(|_| NvmeDeviceError::DmaAllocationFailed)?;
    Ok(u64::from_le_bytes(entry))
}

/// Returns the maximum transfer size (in bytes) supported by a single PRP chain from `prp1`.
pub(crate) fn max_transfer_bytes(prp1: u64) -> usize {
    let page_size = PAGE_SIZE as u64;
    let page_mask = page_size - 1;
    let offset = prp1 & page_mask;
    let first_page_bytes = (page_size - offset) as usize;
    first_page_bytes.saturating_add(PRP_ENTRIES_PER_PAGE * PAGE_SIZE)
}

fn write_prp_entry(list: &DmaCoherent, index: usize, addr: u64) -> Result<(), NvmeDeviceError> {
    debug_assert!(index < PRP_ENTRIES_PER_PAGE);
    list.writer()
        .skip(index * size_of::<u64>())
        .write_val(&addr)
        .map_err(|_| NvmeDeviceError::DmaAllocationFailed)
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn single_page_transfer_uses_prp1_only() {
        let dma = DmaCoherent::alloc(1, true).unwrap();
        let addr = dma.daddr() as u64;
        let prp = PrpPointers::build_prp(addr, PAGE_SIZE / 2).unwrap();
        assert_eq!(prp.prp1, addr);
        assert_eq!(prp.prp2, 0);
        assert!(prp.list_pages.is_empty());
    }

    #[ktest]
    fn two_page_transfer_uses_prp2() {
        let dma = DmaCoherent::alloc(2, true).unwrap();
        let addr = dma.daddr() as u64;
        let prp = PrpPointers::build_prp(addr, PAGE_SIZE * 2).unwrap();
        assert_eq!(prp.prp1, addr);
        assert_eq!(prp.prp2, addr + PAGE_SIZE as u64);
        assert!(prp.list_pages.is_empty());
    }

    #[ktest]
    fn four_page_transfer_fills_prp_list() {
        let dma = DmaCoherent::alloc(4, true).unwrap();
        let addr = dma.daddr() as u64;
        let prp = PrpPointers::build_prp(addr, PAGE_SIZE * 4).unwrap();
        assert_eq!(prp.list_pages.len(), 1);

        for page_index in 1..4 {
            let mut entry = [0u8; 8];
            prp.list_pages[0]
                .read_bytes((page_index - 1) * 8, &mut entry)
                .unwrap();
            assert_eq!(
                u64::from_le_bytes(entry),
                addr + page_index as u64 * PAGE_SIZE as u64
            );
        }
    }

    fn read_list_entry(list: &DmaCoherent, index: usize) -> u64 {
        let mut entry = [0u8; 8];
        list.read_bytes(index * size_of::<u64>(), &mut entry)
            .unwrap();
        u64::from_le_bytes(entry)
    }

    #[ktest]
    fn six_hundred_page_transfer_chains_prp_list() {
        const NR_PAGES: u64 = 600;

        let dma = DmaCoherent::alloc(1, true).unwrap();
        let addr = dma.daddr() as u64;
        let prp = PrpPointers::build_prp(addr, (NR_PAGES as usize) * PAGE_SIZE).unwrap();

        assert_eq!(prp.prp1, addr);
        assert_eq!(prp.prp2, prp.list_pages()[0].daddr() as u64);
        assert_eq!(prp.list_pages().len(), 2);

        let first_list = &prp.list_pages()[0];
        let second_list = &prp.list_pages()[1];

        for page_index in 1..512 {
            assert_eq!(
                read_list_entry(first_list, (page_index - 1) as usize),
                addr + page_index * PAGE_SIZE as u64
            );
        }
        assert_eq!(
            read_list_entry(first_list, PRP_ENTRIES_PER_PAGE - 1),
            second_list.daddr() as u64
        );
        assert_eq!(
            read_list_entry(second_list, 0),
            addr + 512 * PAGE_SIZE as u64
        );
        for page_index in 513..NR_PAGES {
            assert_eq!(
                read_list_entry(second_list, (page_index - 512) as usize),
                addr + page_index * PAGE_SIZE as u64
            );
        }
    }
}
