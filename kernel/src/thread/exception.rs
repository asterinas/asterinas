// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

use aster_rights::Full;
#[cfg(target_arch = "x86_64")]
use ostd::cpu::context::CpuException;
#[cfg(target_arch = "riscv64")]
use ostd::cpu::context::CpuExceptionInfo as CpuException;
#[cfg(target_arch = "loongarch64")]
use ostd::cpu::context::CpuExceptionInfo as CpuException;
use ostd::{cpu::context::UserContext, task::Task};

use crate::{
    prelude::*,
    process::signal::signals::fault::FaultSignal,
    vm::{page_fault_handler::PageFaultHandler, perms::VmPerms, vmar::Vmar},
};

/// Page fault information converted from [`CpuExceptionInfo`].
///
/// `From<CpuExceptionInfo>` should be implemented for this struct.
/// If `CpuExceptionInfo` is a page fault, `try_from` should return `Ok(PageFaultInfo)`,
/// or `Err(())` (no error information) otherwise.
pub struct PageFaultInfo {
    /// The virtual address where a page fault occurred.
    pub address: Vaddr,

    /// The [`VmPerms`] required by the memory operation that causes page fault.
    /// For example, a "store" operation may require `VmPerms::WRITE`.
    pub required_perms: VmPerms,
}

/// We can't handle most exceptions, just send self a fault signal before return to user space.
pub fn handle_exception(ctx: &Context, context: &UserContext, exception: CpuException) {
    debug!("[User Trap] handle exception: {:#x?}", exception);

    if let Ok(page_fault_info) = PageFaultInfo::try_from(&exception) {
        let user_space = ctx.user_space();
        let root_vmar = user_space.root_vmar();
        if handle_page_fault_from_vmar(root_vmar, &page_fault_info).is_ok() {
            return;
        }
    }

    generate_fault_signal(exception, ctx);
}

/// Handles the page fault occurs in the input `Vmar`.
fn handle_page_fault_from_vmar(
    root_vmar: &Vmar<Full>,
    page_fault_info: &PageFaultInfo,
) -> core::result::Result<(), ()> {
    if let Err(e) = root_vmar.handle_page_fault(page_fault_info) {
        warn!(
            "page fault handler failed: addr: 0x{:x}, err: {:?}",
            page_fault_info.address, e
        );
        return Err(());
    }
    Ok(())
}

/// generate a fault signal for current process.
fn generate_fault_signal(exception: CpuException, ctx: &Context) {
    let signal = FaultSignal::from(&exception);
    ctx.posix_thread.enqueue_signal(Box::new(signal));
}

pub(super) fn page_fault_handler(info: &CpuException) -> core::result::Result<(), ()> {
    let task = Task::current().unwrap();
    let thread_local = task.as_thread_local().unwrap();

    if thread_local.is_page_fault_disabled() {
        // Do nothing if the page fault handler is disabled. This will typically cause the fallible
        // memory operation to report `EFAULT` errors immediately.
        return Err(());
    }

    let user_space = CurrentUserSpace::new(thread_local);
    handle_page_fault_from_vmar(user_space.root_vmar(), &info.try_into().unwrap())
}
