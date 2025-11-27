// SPDX-License-Identifier: MPL-2.0

use tdx_guest::{tdcall::accept_page, tdvmcall::map_gpa, TdxTrapFrame};

use super::trap::TrapFrame;
use crate::{mm::PAGE_SIZE, prelude::Paddr};

const SHARED_BIT: u8 = 51;
const SHARED_MASK: u64 = 1u64 << SHARED_BIT;

#[derive(Debug)]
pub enum PageConvertError {
    InvalidAddress,
    TdCall,
    TdVmcall,
}

/// Converts physical pages to Intel TDX shared pages.
///
/// Invokes the [`map_gpa`] TDVMCALL to convert those pages into Intel TDX shared pages. Due to
/// the conversion, any existing data on the pages will be erased.
///
/// # Safety
///
/// The caller must ensure that:
///  - The provided physical address range is in bounds, i.e., it should fall within the maximum
///    Guest Physical Address (GPA) limit.
///  - All of the physical pages are untyped memory. Therefore, converting and erasing the data
///    will not cause memory safety issues.
pub unsafe fn unprotect_gpa_range(gpa: Paddr, page_num: usize) -> Result<(), PageConvertError> {
    const PAGE_MASK: usize = PAGE_SIZE - 1;
    if gpa & PAGE_MASK != 0 {
        return Err(PageConvertError::InvalidAddress);
    }

    // FIXME: The `map_gpa` API from the `tdx_guest` crate should have been marked `unsafe`
    // because it has no way to determine if the input physical address is safe or not.
    map_gpa(
        (gpa & (!PAGE_MASK)) as u64 | SHARED_MASK,
        (page_num * PAGE_SIZE) as u64,
    )
    .map_err(|_| PageConvertError::TdVmcall)
}

/// Converts physical pages to Intel TDX private pages.
///
/// Invokes the [`map_gpa`] TDVMCALL and the [`accept_page`] TDCALL to convert those pages
/// into Intel TDX private pages. Due to the conversion, any existing data on the pages will be
/// erased.
///
/// # Safety
///
/// The caller must ensure that:
///  - The provided physical address range is in bounds, i.e., it should fall within the maximum
///    Guest Physical Address (GPA) limit.
///  - All of the physical pages are untyped memory. Therefore, converting and erasing the data
///    will not cause memory safety issues.
pub unsafe fn protect_gpa_range(gpa: Paddr, page_num: usize) -> Result<(), PageConvertError> {
    const PAGE_MASK: usize = PAGE_SIZE - 1;
    if gpa & !PAGE_MASK == 0 {
        return Err(PageConvertError::InvalidAddress);
    }

    // GPA outside the physical frame range cannot be converted to private pages.
    // This limitation can be removed when supporting private I/O pages in the future.
    if gpa + page_num * PAGE_SIZE > crate::mm::frame::max_paddr() {
        return Err(PageConvertError::InvalidAddress);
    }

    map_gpa((gpa & PAGE_MASK) as u64, (page_num * PAGE_SIZE) as u64)
        .map_err(|_| PageConvertError::TdVmcall)?;
    for i in 0..page_num {
        // SAFETY: The caller ensures that the address range represents physical memory so the
        // memory can be accepted.
        unsafe {
            accept_page(0, (gpa + i * PAGE_SIZE) as u64).map_err(|_| PageConvertError::TdCall)?
        };
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
