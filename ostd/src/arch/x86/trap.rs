// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

use core::sync::atomic::{AtomicBool, Ordering};

use align_ext::AlignExt;
use log::debug;
#[cfg(feature = "intel_tdx")]
use tdx_guest::tdcall;
use trapframe::TrapFrame;

#[cfg(feature = "intel_tdx")]
use crate::arch::{cpu::VIRTUALIZATION_EXCEPTION, tdx_guest::handle_virtual_exception};
use crate::{
    cpu::{CpuException, PageFaultErrorCode, PAGE_FAULT},
    cpu_local,
    mm::{
        kspace::{KERNEL_PAGE_TABLE, LINEAR_MAPPING_BASE_VADDR, LINEAR_MAPPING_VADDR_RANGE},
        page_prop::{CachePolicy, PageProperty},
        PageFlags, PrivilegedPageFlags as PrivFlags, PAGE_SIZE,
    },
    trap::call_irq_callback_functions,
};

cpu_local! {
    static IS_KERNEL_INTERRUPTED: AtomicBool = AtomicBool::new(false);
}

/// Returns true if this function is called within the context of an IRQ handler
/// and the IRQ occurs while the CPU is executing in the kernel mode.
/// Otherwise, it returns false.
pub fn is_kernel_interrupted() -> bool {
    IS_KERNEL_INTERRUPTED.load(Ordering::Acquire)
}

/// Only from kernel
#[no_mangle]
extern "sysv64" fn trap_handler(f: &mut TrapFrame) {
    if CpuException::is_cpu_exception(f.trap_num as u16) {
        match CpuException::to_cpu_exception(f.trap_num as u16).unwrap() {
            #[cfg(feature = "intel_tdx")]
            &VIRTUALIZATION_EXCEPTION => {
                let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
                handle_virtual_exception(f, &ve_info);
            }
            &PAGE_FAULT => {
                handle_kernel_page_fault(f);
            }
            exception => {
                panic!(
                    "Cannot handle kernel cpu exception:{:?}. Error code:{:x?}; Trapframe:{:#x?}.",
                    exception, f.error_code, f
                );
            }
        }
    } else {
        IS_KERNEL_INTERRUPTED.store(true, Ordering::Release);
        call_irq_callback_functions(f);
        IS_KERNEL_INTERRUPTED.store(false, Ordering::Release);
    }
}

/// FIXME: this is a hack because we don't allocate kernel space for IO memory. We are currently
/// using the linear mapping for IO memory. This is not a good practice.
fn handle_kernel_page_fault(f: &TrapFrame) {
    let page_fault_vaddr = x86_64::registers::control::Cr2::read().as_u64();
    let error_code = PageFaultErrorCode::from_bits_truncate(f.error_code);
    debug!(
        "kernel page fault: address {:?}, error code {:?}",
        page_fault_vaddr as *const (), error_code
    );

    assert!(
        LINEAR_MAPPING_VADDR_RANGE.contains(&(page_fault_vaddr as usize)),
        "kernel page fault: the address is outside the range of the linear mapping",
    );

    const SUPPORTED_ERROR_CODES: PageFaultErrorCode = PageFaultErrorCode::PRESENT
        .union(PageFaultErrorCode::WRITE)
        .union(PageFaultErrorCode::INSTRUCTION);
    assert!(
        SUPPORTED_ERROR_CODES.contains(error_code),
        "kernel page fault: the error code is not supported",
    );

    assert!(
        !error_code.contains(PageFaultErrorCode::INSTRUCTION),
        "kernel page fault: the direct mapping cannot be executed",
    );
    assert!(
        !error_code.contains(PageFaultErrorCode::PRESENT),
        "kernel page fault: the direct mapping already exists",
    );

    // Do the mapping
    let page_table = KERNEL_PAGE_TABLE
        .get()
        .expect("kernel page fault: the kernel page table is not initialized");
    let vaddr = (page_fault_vaddr as usize).align_down(PAGE_SIZE);
    let paddr = vaddr - LINEAR_MAPPING_BASE_VADDR;

    // SAFETY:
    // 1. We have checked that the page fault address falls within the address range of the direct
    //    mapping of physical memory.
    // 2. We map the address to the correct physical page with the correct flags, where the
    //    correctness follows the semantics of the direct mapping of physical memory.
    unsafe {
        page_table
            .map(
                &(vaddr..vaddr + PAGE_SIZE),
                &(paddr..paddr + PAGE_SIZE),
                PageProperty {
                    flags: PageFlags::RW,
                    cache: CachePolicy::Uncacheable,
                    #[cfg(not(feature = "intel_tdx"))]
                    priv_flags: PrivFlags::GLOBAL,
                    #[cfg(feature = "intel_tdx")]
                    priv_flags: PrivFlags::SHARED | PrivFlags::GLOBAL,
                },
            )
            .unwrap();
    }
}
