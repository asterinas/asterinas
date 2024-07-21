// SPDX-License-Identifier: MPL-2.0

//! General trap handling in OSTD.

use core::sync::atomic::{AtomicBool, Ordering};

use align_ext::AlignExt;
use log::debug;
#[cfg(feature = "intel_tdx")]
use tdx_guest::{tdcall, tdx_is_enabled};
use trapframe::TrapFrame;

#[cfg(feature = "intel_tdx")]
use crate::arch::tdx_guest::handle_virtual_exception;
use crate::{
    arch::{cpu::SYSCALL_EXCEPTION_NUM, ex_table::ExTable, irq::IRQ_LIST},
    cpu::{CpuException, CpuExceptionInfo, CpuExceptionType, PageFaultErrorCode, UserContext},
    cpu_local,
    mm::{
        kspace::{KERNEL_PAGE_TABLE, LINEAR_MAPPING_BASE_VADDR, LINEAR_MAPPING_VADDR_RANGE},
        page_prop::{CachePolicy, PageProperty},
        PageFlags, PrivilegedPageFlags as PrivFlags, MAX_USERSPACE_VADDR, PAGE_SIZE,
    },
    task::current_task,
    user::{ReturnReason, UserContextApiInternal},
};

cpu_local! {
    static IN_INTERRUPT_CONTEXT: AtomicBool = AtomicBool::new(false);
}

/// Returns true if this function is called within the context of an IRQ handler
/// and the IRQ occurs while the CPU is executing in the kernel mode.
/// Otherwise, it returns false.
///
/// FIXME: For ISAs that supports re-entrant interrupts, we may need to record nested level here.
pub fn in_interrupt_context() -> bool {
    IN_INTERRUPT_CONTEXT.load(Ordering::Acquire)
}

// For x86 CPUs, interrupts are not re-entrant. Local interrupts will be disabled when
// an interrupt handler is called (unless interrupts are re-enabled in an interrupt handler).
fn re_enable_interrupts() {
    #[cfg(target_arch = "x86_64")]
    {
        debug_assert!(!crate::arch::irq::is_local_enabled());
        crate::arch::irq::enable_local();
    }
}

/// Handles trap originated from the user mode.
///
/// It returns whether we can immediately return to the user mode without any
/// further handling. If so, the return value is `None`. Otherwise it returns
/// the reason why the user traps into the kernel mode. The caller can use it
/// to decide how to further handle the trap.
pub(crate) fn user_mode_exception_handler(ctx: &mut UserContext) -> Option<ReturnReason> {
    let trap_frame = ctx.as_trap_frame();
    let exception = CpuException::from_num(trap_frame.trap_num as u16);

    let panic_on_unrecoverable_exception = || {
        panic!(
            "Unrecoverable CPU exception originated from user: {:?}. Context: {:#x?}.",
            exception, ctx,
        );
    };

    let reaction = match exception {
        #[cfg(feature = "intel_tdx")]
        CpuException::VirtualizationException => {
            let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
            handle_virtual_exception(ctx.general_regs_mut(), &ve_info);
            None
        }
        CpuException::NotExplicitInISA(n) => {
            if n == SYSCALL_EXCEPTION_NUM {
                Some(ReturnReason::UserSyscall)
            } else {
                match exception.get_type() {
                    CpuExceptionType::Interrupt => {
                        IN_INTERRUPT_CONTEXT.store(true, Ordering::Release);
                        handle_interrupt(&trap_frame);
                        IN_INTERRUPT_CONTEXT.store(false, Ordering::Release);
                        // Whether to re-enable local interrupts is handled by the interrupt handler.
                        // Just return in this case.
                        return None;
                    }
                    _ => panic_on_unrecoverable_exception(),
                }
            }
        }
        _ => match exception.get_type() {
            CpuExceptionType::FaultOrTrap | CpuExceptionType::Fault | CpuExceptionType::Trap => {
                Some(ReturnReason::UserException)
            }
            _ => panic_on_unrecoverable_exception(),
        },
    };

    re_enable_interrupts();

    reaction
}

/// The trap handler defined for [`trapframe`].
///
/// This handles only traps happened in the kernel mode.
#[export_name = "trap_handler"]
extern "sysv64" fn kernel_mode_exception_handler(f: &mut TrapFrame) {
    let exception = CpuException::from_num(f.trap_num as u16);
    match exception {
        #[cfg(feature = "intel_tdx")]
        CpuException::VirtualizationException => {
            let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
            handle_virtual_exception(f, &ve_info);
        }
        CpuException::PageFault => {
            // TODO: figure out how to get the page fault address for other ISAs.
            #[cfg(target_arch = "x86_64")]
            let page_fault_addr = x86_64::registers::control::Cr2::read().as_u64();

            // The actual user space implementation should be responsible
            // for providing mechanism to treat the 0 virtual address.
            if (0..MAX_USERSPACE_VADDR).contains(&(page_fault_addr as usize)) {
                handle_user_page_fault_in_kernel(f, page_fault_addr);
            } else {
                handle_kernel_page_fault(f, page_fault_addr);
            }
        }
        _ => match exception.get_type() {
            CpuExceptionType::Interrupt => {
                IN_INTERRUPT_CONTEXT.store(true, Ordering::Release);
                handle_interrupt(f);
                IN_INTERRUPT_CONTEXT.store(false, Ordering::Release);
                // Whether to re-enable local interrupts is handled by the interrupt handler.
                // Just return in this case.
                return;
            }
            _ => panic!(
                "Unrecoverable CPU exception originated from kernel: {:?}. Trapframe: {:#x?}.",
                exception, f
            ),
        },
    }

    re_enable_interrupts();
}

/// Handles page fault from user space.
fn handle_user_page_fault_in_kernel(f: &mut TrapFrame, page_fault_addr: u64) {
    let current_task = current_task().unwrap();
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

    #[cfg(not(feature = "intel_tdx"))]
    let priv_flags = PrivFlags::GLOBAL;
    #[cfg(feature = "intel_tdx")]
    let priv_flags = if tdx_is_enabled() {
        PrivFlags::SHARED | PrivFlags::GLOBAL
    } else {
        PrivFlags::GLOBAL
    };

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

fn handle_interrupt(trap_frame: &TrapFrame) {
    let irq_line = IRQ_LIST.get().unwrap().get(trap_frame.trap_num).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(trap_frame);
    }
    drop(callback_functions);

    crate::arch::interrupts_ack();

    // The upper half of the interrupt handler is done. We can re-enable local interrupts.
    re_enable_interrupts();

    // Now we can process the pending softirqs.

    crate::exception::softirq::process_pending();
}
