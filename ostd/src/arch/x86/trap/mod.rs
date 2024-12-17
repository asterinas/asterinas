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

mod gdt;
mod idt;
mod syscall;

use align_ext::AlignExt;
use cfg_if::cfg_if;
use log::debug;

use super::ex_table::ExTable;
use crate::{
    cpu::{CpuException, CpuExceptionInfo, PageFaultErrorCode},
    cpu_local_cell,
    mm::{
        kspace::{KERNEL_PAGE_TABLE, LINEAR_MAPPING_BASE_VADDR, LINEAR_MAPPING_VADDR_RANGE},
        page_prop::{CachePolicy, PageProperty},
        PageFlags, PrivilegedPageFlags as PrivFlags, MAX_USERSPACE_VADDR, PAGE_SIZE,
    },
    task::Task,
    trap::call_irq_callback_functions,
};

cfg_if! {
    if #[cfg(feature = "cvm_guest")] {
        use tdx_guest::{tdcall, tdx_is_enabled, handle_virtual_exception};
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
#[allow(missing_docs)]
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

/// Initialize interrupt handling on x86_64.
///
/// # Safety
///
/// This function will:
///
/// - Disable interrupt.
/// - Switch to a new [GDT], extend 7 more entries from the current one.
/// - Switch to a new [TSS], `GSBASE` pointer to its base address.
/// - Switch to a new [IDT], override the current one.
/// - Enable [`syscall`] instruction.
///     - set `EFER::SYSTEM_CALL_EXTENSIONS`
///
/// [GDT]: https://wiki.osdev.org/GDT
/// [IDT]: https://wiki.osdev.org/IDT
/// [TSS]: https://wiki.osdev.org/Task_State_Segment
/// [`syscall`]: https://www.felixcloutier.com/x86/syscall
///
#[cfg(any(target_os = "none", target_os = "uefi"))]
pub unsafe fn init(on_bsp: bool) {
    x86_64::instructions::interrupts::disable();
    gdt::init(on_bsp);
    idt::init();
    syscall::init();
}

/// User space context.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
#[allow(missing_docs)]
pub struct UserContext {
    pub general: GeneralRegs,
    pub trap_num: usize,
    pub error_code: usize,
}

/// General registers.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
#[allow(missing_docs)]
pub struct GeneralRegs {
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
    pub rip: usize,
    pub rflags: usize,
    pub fsbase: usize,
    pub gsbase: usize,
}

impl UserContext {
    /// Get number of syscall.
    pub fn get_syscall_num(&self) -> usize {
        self.general.rax
    }

    /// Get return value of syscall.
    pub fn get_syscall_ret(&self) -> usize {
        self.general.rax
    }

    /// Set return value of syscall.
    pub fn set_syscall_ret(&mut self, ret: usize) {
        self.general.rax = ret;
    }

    /// Get syscall args.
    pub fn get_syscall_args(&self) -> [usize; 6] {
        [
            self.general.rdi,
            self.general.rsi,
            self.general.rdx,
            self.general.r10,
            self.general.r8,
            self.general.r9,
        ]
    }

    /// Set instruction pointer.
    pub fn set_ip(&mut self, ip: usize) {
        self.general.rip = ip;
    }

    /// Set stack pointer.
    pub fn set_sp(&mut self, sp: usize) {
        self.general.rsp = sp;
    }

    /// Get stack pointer.
    pub fn get_sp(&self) -> usize {
        self.general.rsp
    }

    /// Set thread-local storage pointer.
    pub fn set_tls(&mut self, tls: usize) {
        self.general.fsbase = tls;
    }
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
    match CpuException::to_cpu_exception(f.trap_num as u16) {
        #[cfg(feature = "cvm_guest")]
        Some(CpuException::VIRTUALIZATION_EXCEPTION) => {
            let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
            let mut trapframe_wrapper = TrapFrameWrapper(&mut *f);
            handle_virtual_exception(&mut trapframe_wrapper, &ve_info);
            *f = *trapframe_wrapper.0;
        }
        Some(CpuException::PAGE_FAULT) => {
            let page_fault_addr = x86_64::registers::control::Cr2::read_raw();
            // The actual user space implementation should be responsible
            // for providing mechanism to treat the 0 virtual address.
            if (0..MAX_USERSPACE_VADDR).contains(&(page_fault_addr as usize)) {
                handle_user_page_fault(f, page_fault_addr);
            } else {
                handle_kernel_page_fault(f, page_fault_addr);
            }
        }
        Some(exception) => {
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

/// Handles page fault from user space.
fn handle_user_page_fault(f: &mut TrapFrame, page_fault_addr: u64) {
    let current_task = Task::current().unwrap();
    let user_space = current_task
        .user_space()
        .expect("the user space is missing when a page fault from the user happens.");

    let info = CpuExceptionInfo {
        page_fault_addr: page_fault_addr as usize,
        id: f.trap_num,
        error_code: f.error_code,
    };

    let res = user_space.vm_space().handle_page_fault(&info);
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
fn handle_kernel_page_fault(f: &TrapFrame, page_fault_vaddr: u64) {
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

    cfg_if! {
        if #[cfg(feature = "cvm_guest")] {
            let priv_flags = if tdx_is_enabled() {
                PrivFlags::SHARED | PrivFlags::GLOBAL
            } else {
                PrivFlags::GLOBAL
            };
        } else {
            let priv_flags = PrivFlags::GLOBAL;
        }
    }

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
                    priv_flags,
                },
            )
            .unwrap();
    }
}
