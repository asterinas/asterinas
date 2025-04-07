// SPDX-License-Identifier: MPL-2.0 OR MIT
//
// The original source code is from [trapframe-rs](https://github.com/rcore-os/trapframe-rs),
// which is released under the following license:
//
// SPDX-License-Identifier: MIT
//
// Copyright (c) 2020 - 2024 Runji Wang
//
// We make the following new changes:
// * Implement the `trap_handler` of Asterinas.
//
// These changes are released under the following license:
//
// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

pub(super) mod gdt;
mod idt;
mod syscall;

use align_ext::AlignExt;
use cfg_if::cfg_if;
use log::debug;
use spin::Once;

use super::{cpu::context::GeneralRegs, ex_table::ExTable};
use crate::{
    arch::{
        if_tdx_enabled,
        irq::{disable_local, enable_local},
    },
    cpu::context::{CpuException, PageFaultErrorCode, RawPageFaultInfo},
    cpu_local_cell,
    mm::{
        kspace::{KERNEL_PAGE_TABLE, LINEAR_MAPPING_BASE_VADDR, LINEAR_MAPPING_VADDR_RANGE},
        page_prop::{CachePolicy, PageProperty},
        PageFlags, PrivilegedPageFlags as PrivFlags, MAX_USERSPACE_VADDR, PAGE_SIZE,
    },
    task::disable_preempt,
    trap::call_irq_callback_functions,
};

cfg_if! {
    if #[cfg(feature = "cvm_guest")] {
        use tdx_guest::{tdcall, handle_virtual_exception};
        use crate::arch::tdx_guest::TrapFrameWrapper;
    }
}

cpu_local_cell! {
    static KERNEL_INTERRUPT_NESTED_LEVEL: u8 = 0;
}

/// Trap frame of kernel interrupt
///
/// # Trap handler
///
/// You need to define a handler function like this:
///
/// ```
/// #[no_mangle]
/// extern "sysv64" fn trap_handler(tf: &mut TrapFrame) {
///     match tf.trap_num {
///         3 => {
///             println!("TRAP: BreakPoint");
///             tf.rip += 1;
///         }
///         _ => panic!("TRAP: {:#x?}", tf),
///     }
/// }
/// ```
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
#[expect(missing_docs)]
pub struct TrapFrame {
    // Pushed by 'trap.S'
    pub rax: usize,
    pub rbx: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rbp: usize,
    pub rsp: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
    pub _pad: usize,

    pub trap_num: usize,
    pub error_code: usize,

    // Pushed by CPU
    pub rip: usize,
    pub cs: usize,
    pub rflags: usize,
}

/// Initializes interrupt handling on x86_64.
///
/// This function will:
/// - Switch to a new, CPU-local [GDT].
/// - Switch to a new, CPU-local [TSS].
/// - Switch to a new, global [IDT].
/// - Enable the [`syscall`] instruction.
///
/// [GDT]: https://wiki.osdev.org/GDT
/// [IDT]: https://wiki.osdev.org/IDT
/// [TSS]: https://wiki.osdev.org/Task_State_Segment
/// [`syscall`]: https://www.felixcloutier.com/x86/syscall
///
/// # Safety
///
/// This method must be called only in the boot context of each available processor.
pub(crate) unsafe fn init() {
    // SAFETY: We're in the boot context, so no preemption can occur.
    unsafe { gdt::init() };

    idt::init();

    // SAFETY: `gdt::init` has been called before.
    unsafe { syscall::init() };
}

/// Userspace context.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub(super) struct RawUserContext {
    pub(super) general: GeneralRegs,
    pub(super) trap_num: usize,
    pub(super) error_code: usize,
}

/// Returns true if this function is called within the context of an IRQ handler
/// and the IRQ occurs while the CPU is executing in the kernel mode.
/// Otherwise, it returns false.
pub fn is_kernel_interrupted() -> bool {
    KERNEL_INTERRUPT_NESTED_LEVEL.load() != 0
}

/// Handle traps (only from kernel).
#[no_mangle]
extern "sysv64" fn trap_handler(f: &mut TrapFrame) {
    fn enable_local_if(cond: bool) {
        if cond {
            enable_local();
        }
    }

    fn disable_local_if(cond: bool) {
        if cond {
            disable_local();
        }
    }

    // The IRQ state before trapping. We need to ensure that the IRQ state
    // during exception handling is consistent with the state before the trap.
    let was_irq_enabled =
        f.rflags as u64 & x86_64::registers::rflags::RFlags::INTERRUPT_FLAG.bits() > 0;

    let cpu_exception = CpuException::new(f.trap_num, f.error_code);
    match cpu_exception {
        #[cfg(feature = "cvm_guest")]
        Some(CpuException::VirtualizationException) => {
            let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
            // We need to enable interrupts only after `tdcall::get_veinfo` is called
            // to avoid nested `#VE`s.
            enable_local_if(was_irq_enabled);
            let mut trapframe_wrapper = TrapFrameWrapper(&mut *f);
            handle_virtual_exception(&mut trapframe_wrapper, &ve_info);
            *f = *trapframe_wrapper.0;
            disable_local_if(was_irq_enabled);
        }
        Some(CpuException::PageFault(raw_page_fault_info)) => {
            enable_local_if(was_irq_enabled);
            // The actual user space implementation should be responsible
            // for providing mechanism to treat the 0 virtual address.
            if (0..MAX_USERSPACE_VADDR).contains(&raw_page_fault_info.addr) {
                handle_user_page_fault(f, cpu_exception.as_ref().unwrap());
            } else {
                handle_kernel_page_fault(raw_page_fault_info);
            }
            disable_local_if(was_irq_enabled);
        }
        Some(exception) => {
            enable_local_if(was_irq_enabled);
            panic!(
                "cannot handle kernel CPU exception: {:?}, trapframe: {:?}",
                exception, f
            );
        }
        None => {
            KERNEL_INTERRUPT_NESTED_LEVEL.add_assign(1);
            call_irq_callback_functions(f, f.trap_num);
            KERNEL_INTERRUPT_NESTED_LEVEL.sub_assign(1);
        }
    }
}

#[expect(clippy::type_complexity)]
static USER_PAGE_FAULT_HANDLER: Once<fn(&CpuException) -> core::result::Result<(), ()>> =
    Once::new();

/// Injects a custom handler for page faults that occur in the kernel and
/// are caused by user-space address.
pub fn inject_user_page_fault_handler(
    handler: fn(info: &CpuException) -> core::result::Result<(), ()>,
) {
    USER_PAGE_FAULT_HANDLER.call_once(|| handler);
}

/// Handles page fault from user space.
fn handle_user_page_fault(f: &mut TrapFrame, exception: &CpuException) {
    let handler = USER_PAGE_FAULT_HANDLER
        .get()
        .expect("a page fault handler is missing");

    let res = handler(exception);
    // Copying bytes by bytes can recover directly
    // if handling the page fault successfully.
    if res.is_ok() {
        return;
    }

    // Use the exception table to recover to normal execution.
    if let Some(addr) = ExTable::find_recovery_inst_addr(f.rip) {
        f.rip = addr;
    } else {
        panic!("Cannot handle user page fault; Trapframe:{:#x?}.", f);
    }
}

/// FIXME: this is a hack because we don't allocate kernel space for IO memory. We are currently
/// using the linear mapping for IO memory. This is not a good practice.
fn handle_kernel_page_fault(info: RawPageFaultInfo) {
    let preempt_guard = disable_preempt();

    let RawPageFaultInfo {
        error_code,
        addr: page_fault_vaddr,
    } = info;
    debug!(
        "kernel page fault: address {:?}, error code {:?}",
        page_fault_vaddr as *const (), error_code
    );

    assert!(
        LINEAR_MAPPING_VADDR_RANGE.contains(&page_fault_vaddr),
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
    let vaddr = page_fault_vaddr.align_down(PAGE_SIZE);
    let paddr = vaddr - LINEAR_MAPPING_BASE_VADDR;

    let priv_flags = if_tdx_enabled!({
        PrivFlags::SHARED | PrivFlags::GLOBAL
    } else {
        PrivFlags::GLOBAL
    });
    let prop = PageProperty {
        has_map: true,
        flags: PageFlags::RW,
        cache: CachePolicy::Uncacheable,
        priv_flags,
    };

    let mut cursor = page_table
        .cursor_mut(&preempt_guard, &(vaddr..vaddr + PAGE_SIZE))
        .unwrap();

    // SAFETY:
    // 1. We have checked that the page fault address falls within the address range of the direct
    //    mapping of physical memory.
    // 2. We map the address to the correct physical page with the correct flags, where the
    //    correctness follows the semantics of the direct mapping of physical memory.
    unsafe { cursor.map(crate::mm::kspace::MappedItem::Untracked(paddr, 1, prop)) }.unwrap();
}
