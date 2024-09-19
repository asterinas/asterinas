// SPDX-License-Identifier: MPL-2.0

use log::warn;
use tdx_guest::{tdcall::accept_page, tdvmcall::map_gpa, TdxTrapFrame};

use crate::{
    mm::{
        kspace::KERNEL_PAGE_TABLE,
        paddr_to_vaddr,
        page_prop::{PageProperty, PrivilegedPageFlags as PrivFlags},
        page_table::boot_pt,
        PAGE_SIZE,
    },
    prelude::Paddr,
    trap::TrapFrame,
};

const SHARED_BIT: u8 = 51;
const SHARED_MASK: u64 = 1u64 << SHARED_BIT;

#[derive(Debug)]
pub enum PageConvertError {
    PageTable,
    TdCall,
    TdVmcall,
}

/// Sets the given physical address range to Intel TDX shared pages.
/// Clears the data within the given address range.
/// Make sure the provided physical address is page size aligned.
///
/// # Safety
///
/// To safely use this function, the caller must ensure that:
/// - The given guest physical address range is currently mapped in the page table.
/// - The `page_num` argument represents a valid number of pages.
/// - This function will erase any valid data in the range and should not assume that the data will still be there after the operation.
pub unsafe fn unprotect_gpa_range(gpa: Paddr, page_num: usize) -> Result<(), PageConvertError> {
    const PAGE_MASK: usize = PAGE_SIZE - 1;
    if gpa & PAGE_MASK != 0 {
        warn!("Misaligned address: {:x}", gpa);
    }

    // Protect the page in the boot page table if in the boot phase.
    let protect_op = |prop: &mut PageProperty| {
        *prop = PageProperty {
            flags: prop.flags,
            cache: prop.cache,
            priv_flags: prop.priv_flags | PrivFlags::SHARED,
        }
    };
    let _ = boot_pt::with_borrow(|boot_pt| {
        for i in 0..page_num {
            let vaddr = paddr_to_vaddr(gpa + i * PAGE_SIZE);
            boot_pt.protect_base_page(vaddr, protect_op);
        }
    });
    // Protect the page in the kernel page table.
    let pt = KERNEL_PAGE_TABLE.get().unwrap();
    let vaddr = paddr_to_vaddr(gpa);
    pt.protect_flush_tlb(&(vaddr..vaddr + page_num * PAGE_SIZE), protect_op)
        .map_err(|_| PageConvertError::PageTable)?;

    map_gpa(
        (gpa & (!PAGE_MASK)) as u64 | SHARED_MASK,
        (page_num * PAGE_SIZE) as u64,
    )
    .map_err(|_| PageConvertError::TdVmcall)
}

/// Sets the given physical address range to Intel TDX private pages.
/// Make sure the provided physical address is page size aligned.
///
/// # Safety
///
/// To safely use this function, the caller must ensure that:
/// - The given guest physical address range is currently mapped in the page table.
/// - The `page_num` argument represents a valid number of pages.
///
pub unsafe fn protect_gpa_range(gpa: Paddr, page_num: usize) -> Result<(), PageConvertError> {
    const PAGE_MASK: usize = PAGE_SIZE - 1;
    if gpa & !PAGE_MASK == 0 {
        warn!("Misaligned address: {:x}", gpa);
    }

    // Protect the page in the boot page table if in the boot phase.
    let protect_op = |prop: &mut PageProperty| {
        *prop = PageProperty {
            flags: prop.flags,
            cache: prop.cache,
            priv_flags: prop.priv_flags - PrivFlags::SHARED,
        }
    };
    let _ = boot_pt::with_borrow(|boot_pt| {
        for i in 0..page_num {
            let vaddr = paddr_to_vaddr(gpa + i * PAGE_SIZE);
            boot_pt.protect_base_page(vaddr, protect_op);
        }
    });
    // Protect the page in the kernel page table.
    let pt = KERNEL_PAGE_TABLE.get().unwrap();
    let vaddr = paddr_to_vaddr(gpa);
    pt.protect_flush_tlb(&(vaddr..vaddr + page_num * PAGE_SIZE), protect_op)
        .map_err(|_| PageConvertError::PageTable)?;

    map_gpa((gpa & PAGE_MASK) as u64, (page_num * PAGE_SIZE) as u64)
        .map_err(|_| PageConvertError::TdVmcall)?;
    for i in 0..page_num {
        unsafe {
            accept_page(0, (gpa + i * PAGE_SIZE) as u64).map_err(|_| PageConvertError::TdCall)?;
        }
    }
    Ok(())
}

pub struct TrapFrameWrapper<'a>(pub &'a mut TrapFrame);

#[cfg(feature = "cvm_guest")]
impl TdxTrapFrame for TrapFrameWrapper<'_> {
    fn rax(&self) -> usize {
        self.0.rax
    }
    fn set_rax(&mut self, rax: usize) {
        self.0.rax = rax;
    }
    fn rbx(&self) -> usize {
        self.0.rbx
    }
    fn set_rbx(&mut self, rbx: usize) {
        self.0.rbx = rbx;
    }
    fn rcx(&self) -> usize {
        self.0.rcx
    }
    fn set_rcx(&mut self, rcx: usize) {
        self.0.rcx = rcx;
    }
    fn rdx(&self) -> usize {
        self.0.rdx
    }
    fn set_rdx(&mut self, rdx: usize) {
        self.0.rdx = rdx;
    }
    fn rsi(&self) -> usize {
        self.0.rsi
    }
    fn set_rsi(&mut self, rsi: usize) {
        self.0.rsi = rsi;
    }
    fn rdi(&self) -> usize {
        self.0.rdi
    }
    fn set_rdi(&mut self, rdi: usize) {
        self.0.rdi = rdi;
    }
    fn rip(&self) -> usize {
        self.0.rip
    }
    fn set_rip(&mut self, rip: usize) {
        self.0.rip = rip;
    }
    fn r8(&self) -> usize {
        self.0.r8
    }
    fn set_r8(&mut self, r8: usize) {
        self.0.r8 = r8;
    }
    fn r9(&self) -> usize {
        self.0.r9
    }
    fn set_r9(&mut self, r9: usize) {
        self.0.r9 = r9;
    }
    fn r10(&self) -> usize {
        self.0.r10
    }
    fn set_r10(&mut self, r10: usize) {
        self.0.r10 = r10;
    }
    fn r11(&self) -> usize {
        self.0.r11
    }
    fn set_r11(&mut self, r11: usize) {
        self.0.r11 = r11;
    }
    fn r12(&self) -> usize {
        self.0.r12
    }
    fn set_r12(&mut self, r12: usize) {
        self.0.r12 = r12;
    }
    fn r13(&self) -> usize {
        self.0.r13
    }
    fn set_r13(&mut self, r13: usize) {
        self.0.r13 = r13;
    }
    fn r14(&self) -> usize {
        self.0.r14
    }
    fn set_r14(&mut self, r14: usize) {
        self.0.r14 = r14;
    }
    fn r15(&self) -> usize {
        self.0.r15
    }
    fn set_r15(&mut self, r15: usize) {
        self.0.r15 = r15;
    }
    fn rbp(&self) -> usize {
        self.0.rbp
    }
    fn set_rbp(&mut self, rbp: usize) {
        self.0.rbp = rbp;
    }
}
