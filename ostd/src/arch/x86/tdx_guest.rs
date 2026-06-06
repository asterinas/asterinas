// SPDX-License-Identifier: MPL-2.0

use tdx_guest::{
    SHARED_BIT, SHARED_MASK, TdxTrapFrame,
    tdcall::{TdCallError, accept_page},
    tdvmcall::{TdVmcallError, map_gpa},
};

use super::trap::TrapFrame;
use crate::{mm::PAGE_SIZE, prelude::Paddr};

/// Converts physical pages to Intel TDX shared pages.
///
/// It invokes the [`map_gpa`] TDVMCALL to convert those pages into Intel TDX
/// shared pages. Due to the conversion, any existing data on the pages will
/// be lost.
///
/// # Safety
///
/// The caller must ensure that:
///  - The provided physical address is page aligned.
///  - The provided physical address range is in bounds, i.e., it should fall
///    within the maximum Guest Physical Address (GPA) limit.
///  - All of the physical pages are untyped memory. Therefore, converting and
///    erasing the data will not cause memory safety issues.
pub unsafe fn unprotect_gpa_tdvm_call(gpa: Paddr, size: usize) -> Result<(), PageConvertError> {
    debug_assert!(gpa.is_multiple_of(PAGE_SIZE));
    debug_assert!(size.is_multiple_of(PAGE_SIZE));

    // SAFETY: The caller ensures the safety of this operation.
    unsafe { convert_gpa_range(gpa as u64, (gpa + size) as u64, TargetPageState::Shared) }
}

/// Converts physical pages to Intel TDX private pages.
///
/// It invokes the [`map_gpa`] TDVMCALL and the [`accept_page`] TDCALL to
/// convert those pages into Intel TDX private pages. Due to the conversion,
/// any existing data on the pages will be zeroed.
///
/// # Safety
///
/// The caller must ensure that:
///  - The provided physical address is page aligned.
///  - The provided physical address range is in bounds, i.e., it should fall
///    within the maximum Guest Physical Address (GPA) limit.
///  - All of the physical pages are untyped memory. Therefore, converting and
///    erasing the data will not cause memory safety issues.
pub unsafe fn protect_gpa_tdvm_call(gpa: Paddr, size: usize) -> Result<(), PageConvertError> {
    debug_assert!(gpa.is_multiple_of(PAGE_SIZE));
    debug_assert!(size.is_multiple_of(PAGE_SIZE));

    // SAFETY: The caller ensures the safety of this operation.
    unsafe { convert_gpa_range(gpa as u64, (gpa + size) as u64, TargetPageState::Private)? };
    for page_gpa in (gpa..gpa + size).step_by(PAGE_SIZE) {
        // SAFETY: The caller ensures the safety of this operation.
        unsafe { accept_page(0, page_gpa as u64).map_err(PageConvertError::TdCall)? };
    }

    Ok(())
}

#[derive(Debug)]
pub enum PageConvertError {
    #[expect(dead_code)]
    TdCall(TdCallError),
    #[expect(dead_code)]
    TdVmcall {
        output_value: u64,
        error: TdVmcallError,
        retry_count: usize,
    },
}

#[derive(Clone, Copy)]
enum TargetPageState {
    Private,
    Shared,
}

impl TargetPageState {
    fn as_gpa_mask(self) -> u64 {
        match self {
            Self::Private => 0,
            Self::Shared => {
                const { assert!(SHARED_MASK == 1u64 << SHARED_BIT) };
                SHARED_MASK
            }
        }
    }
}

/// # Safety
///
/// The caller must ensure that `start_gpa..end_gpa` represents a valid GPA
/// region that can be safely converted to `target_state`.
unsafe fn convert_gpa_range(
    start_gpa: u64,
    end_gpa: u64,
    target_state: TargetPageState,
) -> Result<(), PageConvertError> {
    // Retrying the same page a second time should succeed; use 3 just in case.
    const MAX_MAP_GPA_RETRIES_PER_PAGE: usize = 3;

    let mut next_gpa = start_gpa;
    let mut retry_count = 0;

    loop {
        let gpa_with_mask = next_gpa | target_state.as_gpa_mask();
        let remaining_size = end_gpa - next_gpa;

        match map_gpa(gpa_with_mask, remaining_size) {
            Ok(()) => return Ok(()),
            Err((retry_gpa, TdVmcallError::TdxRetry))
                if (next_gpa..end_gpa).contains(&retry_gpa)
                    && retry_gpa.is_multiple_of(PAGE_SIZE as u64) =>
            {
                if retry_gpa == next_gpa {
                    retry_count += 1;
                    if retry_count >= MAX_MAP_GPA_RETRIES_PER_PAGE {
                        return Err(PageConvertError::TdVmcall {
                            output_value: retry_gpa,
                            error: TdVmcallError::TdxRetry,
                            retry_count,
                        });
                    }
                } else {
                    next_gpa = retry_gpa;
                    retry_count = 0;
                }
            }
            Err((output_value, error)) => {
                return Err(PageConvertError::TdVmcall {
                    output_value,
                    error,
                    retry_count,
                });
            }
        }
    }
}

pub struct TrapFrameWrapper<'a>(pub &'a mut TrapFrame);

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
