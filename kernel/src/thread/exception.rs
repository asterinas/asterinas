// SPDX-License-Identifier: MPL-2.0

#[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
use ostd::arch::cpu::context::CpuException;
#[cfg(target_arch = "loongarch64")]
use ostd::arch::cpu::context::CpuExceptionInfo as CpuException;
use ostd::{arch::cpu::context::UserContext, task::Task};

use crate::{
    prelude::*,
    process::signal::signals::fault::FaultSignal,
    vm::vmar::{PageFaultInfo, Vmar},
};

pub(super) fn handle_exception(ctx: &Context, user_ctx: &UserContext, exception: CpuException) {
    debug!("[User Trap] handle exception: {:#x?}", exception);

    if let Ok(page_fault_info) = PageFaultInfo::try_from(&exception) {
        let user_space = ctx.user_space();
        let vmar = user_space.vmar();
        if handle_page_fault_from_vmar(vmar, &page_fault_info).is_ok() {
            return;
        }
    }

    // We cannot handle most exceptions. Send a fault signal to the current thread before returning
    // to user space.
    generate_fault_signal(exception, ctx, user_ctx);
}

/// Handles the page fault occurs in the VMAR.
fn handle_page_fault_from_vmar(
    vmar: &Vmar,
    page_fault_info: &PageFaultInfo,
) -> core::result::Result<(), ()> {
    if let Err(e) = vmar.handle_page_fault(page_fault_info) {
        warn!(
            "page fault handler failed: info: {:#x?}, err: {:?}",
            page_fault_info, e
        );
        return Err(());
    }
    Ok(())
}

/// A trait that converts CPU exceptions into fault signals.
///
/// This trait should be implemented by architecture-specific code for [`CpuException`].
pub trait ToFaultSignal {
    /// Converts a CPU exception into a fault signal.
    ///
    /// Returns `None` if the exception must be handled earlier and cannot be delivered as a signal.
    ///
    /// POSIX [requires] `SIGILL` and `SIGFPE` to report the address of the faulting instruction,
    /// and `SIGSEGV` and `SIGBUS` to report the address of the faulting memory reference. Linux
    /// behavior, however, is highly architecture-specific and does not always follow POSIX: for
    /// some exceptions, it reports neither the expected code nor the expected address. Linux may
    /// also attach an instruction address to `SIGTRAP`, but this behavior is not consistent across
    /// architectures.
    ///
    /// [requires]: https://pubs.opengroup.org/onlinepubs/009695399/basedefs/signal.h.html
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal>;
}

/// Generates a fault signal for the current thread.
fn generate_fault_signal(exception: CpuException, ctx: &Context, user_ctx: &UserContext) {
    let Some(signal) = exception.to_fault_signal(user_ctx) else {
        panic!("`{:?}` cannot be handled via signals", exception);
    };
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
    handle_page_fault_from_vmar(user_space.vmar(), &info.try_into().unwrap())
}
