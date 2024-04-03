// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use log::debug;
#[cfg(feature = "intel_tdx")]
use tdx_guest::tdcall;
use trapframe::TrapFrame;

#[cfg(feature = "intel_tdx")]
use crate::arch::tdx_guest::{handle_virtual_exception, TdxTrapFrame};
#[cfg(feature = "intel_tdx")]
use crate::arch::{
    mm::PageTableFlags,
    tdx_guest::{handle_virtual_exception, TdxTrapFrame},
};
use crate::{
    arch::{
        irq::IRQ_LIST,
        mm::{is_kernel_vaddr, PageTableEntry, PageTableFlags},
    },
    boot::memory_region::MemoryRegion,
    cpu::{CpuException, PageFaultErrorCode, PAGE_FAULT},
    cpu_local,
    vm::{page_table::PageTableFlagsTrait, PageTable, PHYS_MEM_BASE_VADDR},
};

#[cfg(feature = "intel_tdx")]
impl TdxTrapFrame for TrapFrame {
    fn rax(&self) -> usize {
        self.rax
    }
    fn set_rax(&mut self, rax: usize) {
        self.rax = rax;
    }
    fn rbx(&self) -> usize {
        self.rbx
    }
    fn set_rbx(&mut self, rbx: usize) {
        self.rbx = rbx;
    }
    fn rcx(&self) -> usize {
        self.rcx
    }
    fn set_rcx(&mut self, rcx: usize) {
        self.rcx = rcx;
    }
    fn rdx(&self) -> usize {
        self.rdx
    }
    fn set_rdx(&mut self, rdx: usize) {
        self.rdx = rdx;
    }
    fn rsi(&self) -> usize {
        self.rsi
    }
    fn set_rsi(&mut self, rsi: usize) {
        self.rsi = rsi;
    }
    fn rdi(&self) -> usize {
        self.rdi
    }
    fn set_rdi(&mut self, rdi: usize) {
        self.rdi = rdi;
    }
    fn rip(&self) -> usize {
        self.rip
    }
    fn set_rip(&mut self, rip: usize) {
        self.rip = rip;
    }
    fn r8(&self) -> usize {
        self.r8
    }
    fn set_r8(&mut self, r8: usize) {
        self.r8 = r8;
    }
    fn r9(&self) -> usize {
        self.r9
    }
    fn set_r9(&mut self, r9: usize) {
        self.r9 = r9;
    }
    fn r10(&self) -> usize {
        self.r10
    }
    fn set_r10(&mut self, r10: usize) {
        self.r10 = r10;
    }
    fn r11(&self) -> usize {
        self.r11
    }
    fn set_r11(&mut self, r11: usize) {
        self.r11 = r11;
    }
    fn r12(&self) -> usize {
        self.r12
    }
    fn set_r12(&mut self, r12: usize) {
        self.r12 = r12;
    }
    fn r13(&self) -> usize {
        self.r13
    }
    fn set_r13(&mut self, r13: usize) {
        self.r13 = r13;
    }
    fn r14(&self) -> usize {
        self.r14
    }
    fn set_r14(&mut self, r14: usize) {
        self.r14 = r14;
    }
    fn r15(&self) -> usize {
        self.r15
    }
    fn set_r15(&mut self, r15: usize) {
        self.r15 = r15;
    }
    fn rbp(&self) -> usize {
        self.rbp
    }
    fn set_rbp(&mut self, rbp: usize) {
        self.rbp = rbp;
    }
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
        call_irq_callback_functions(f);
    }
}

pub(crate) fn call_irq_callback_functions(trap_frame: &TrapFrame) {
    // For x86 CPUs, interrupts are not re-entrant. Local interrupts will be disabled when
    // an interrupt handler is called (Unless interrupts are re-enabled in an interrupt handler).
    //
    // FIXME: For arch that supports re-entrant interrupts, we may need to record nested level here.
    IN_INTERRUPT_CONTEXT.store(true, Ordering::Release);

    let irq_line = IRQ_LIST.get().unwrap().get(trap_frame.trap_num).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(trap_frame);
    }
    if !CpuException::is_cpu_exception(trap_frame.trap_num as u16) {
        crate::arch::interrupts_ack();
    }

    IN_INTERRUPT_CONTEXT.store(false, Ordering::Release);
}

cpu_local! {
    static IN_INTERRUPT_CONTEXT: AtomicBool = AtomicBool::new(false);
}

/// Returns whether we are in the interrupt context.
///
/// FIXME: Here only hardware irq is taken into account. According to linux implementation, if
/// we are in softirq context, or bottom half is disabled, this function also returns true.
pub fn in_interrupt_context() -> bool {
    IN_INTERRUPT_CONTEXT.load(Ordering::Acquire)
}

fn handle_kernel_page_fault(f: &TrapFrame) {
    // We only create mapping: `vaddr = paddr + PHYS_OFFSET` in kernel page fault handler.
    let page_fault_vaddr = x86_64::registers::control::Cr2::read().as_u64();
    debug_assert!(is_kernel_vaddr(page_fault_vaddr as usize));

    // Check kernel region
    // FIXME: The modification to the offset mapping of the kernel code and data should not permitted.
    debug_assert!({
        let kernel_region = MemoryRegion::kernel();
        let start = kernel_region.base();
        let end = start + kernel_region.base() + kernel_region.len();
        !((start..end).contains(&(page_fault_vaddr as usize)))
    });

    // Check error code and construct flags
    // FIXME: The `PageFaultErrorCode` may not be the same on other platforms such as RISC-V.
    let error_code = PageFaultErrorCode::from_bits_truncate(f.error_code);
    debug!(
        "Handling kernel page fault. Page fault address:{:x?}; Error code:{:?}",
        page_fault_vaddr, error_code
    );
    debug_assert!(!error_code.contains(PageFaultErrorCode::USER));
    debug_assert!(!error_code.contains(PageFaultErrorCode::INSTRUCTION));
    let mut flags = PageTableFlags::empty()
        .set_present(true)
        .set_executable(false);
    #[cfg(feature = "intel_tdx")]
    {
        // FIXME: Adding shared bit directly will have security issues.
        flags = flags | PageTableFlags::SHARED;
    }
    if error_code.contains(PageFaultErrorCode::WRITE) {
        flags = flags.set_writable(true);
    }

    // Handle page fault
    let mut page_table: PageTable<PageTableEntry, crate::vm::page_table::KernelMode> =
        unsafe { PageTable::from_root_register() };
    if error_code.contains(PageFaultErrorCode::PRESENT) {
        // FIXME: We should define the initialize mapping and the protect method here should not change the
        // permission of the initialize mapping.
        //
        // Safety: The page fault address has been checked and the flags is constructed based on error code.
        unsafe {
            page_table
                .protect(page_fault_vaddr as usize, flags)
                .unwrap();
        }
    } else {
        // Safety: The page fault address has been checked and the flags is constructed based on error code.
        let paddr = page_fault_vaddr as usize - PHYS_MEM_BASE_VADDR;
        unsafe {
            page_table
                .map(page_fault_vaddr as usize, paddr, flags)
                .unwrap();
        }
    }
}
