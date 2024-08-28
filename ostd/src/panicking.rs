// SPDX-License-Identifier: MPL-2.0

//! Panic support.

use core::ffi::c_void;

use crate::{
    arch::qemu::{exit_qemu, QemuExitCode},
    cpu_local_cell, early_print, early_println,
    sync::SpinLock,
};

extern crate cfg_if;
extern crate gimli;
use gimli::Register;
use unwinding::abi::{
    UnwindContext, UnwindReasonCode, _Unwind_Backtrace, _Unwind_FindEnclosingFunction,
    _Unwind_GetGR, _Unwind_GetIP,
};

cpu_local_cell! {
    static IN_PANIC: bool = false;
}

/// The panic handler must be defined in the binary crate or in the crate that the binary
/// crate explicitly declares by `extern crate`. We cannot let the base crate depend on OSTD
/// due to prismatic dependencies. That's why we export this symbol and state the
/// panic handler in the binary crate.
#[export_name = "__aster_panic_handler"]
pub fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    let _irq_guard = crate::trap::disable_local();

    if IN_PANIC.load() {
        early_println!("{}", info);
        early_println!("The panic handler panicked when processing the above panic. Aborting.");
        abort();
    }

    // If in ktest, we would like to catch the panics and resume the test.
    #[cfg(ktest)]
    {
        use alloc::{boxed::Box, string::ToString};

        use unwinding::panic::begin_panic;

        let throw_info = ostd_test::PanicInfo {
            message: info.message().to_string(),
            file: info.location().unwrap().file().to_string(),
            line: info.location().unwrap().line() as usize,
            col: info.location().unwrap().column() as usize,
            resolve_panic: || {
                IN_PANIC.store(false);
            },
        };
        // Throw an exception and expecting it to be caught.
        begin_panic(Box::new(throw_info.clone()));
    }
    early_println!("{}", info);
    print_stack_trace();
    abort();
}

/// Aborts the QEMU
pub fn abort() -> ! {
    exit_qemu(QemuExitCode::Failed);
}

fn print_stack_trace() {
    /// We acquire a global lock to prevent the frames in the stack trace from
    /// interleaving. The spin lock is used merely for its simplicity.
    static BACKTRACE_PRINT_LOCK: SpinLock<()> = SpinLock::new(());
    let _lock = BACKTRACE_PRINT_LOCK.lock();

    early_println!("Printing stack trace:");

    struct CallbackData {
        counter: usize,
    }
    extern "C" fn callback(unwind_ctx: &UnwindContext<'_>, arg: *mut c_void) -> UnwindReasonCode {
        let data = unsafe { &mut *(arg as *mut CallbackData) };
        data.counter += 1;
        let pc = _Unwind_GetIP(unwind_ctx);
        if pc > 0 {
            let fde_initial_address = _Unwind_FindEnclosingFunction(pc as *mut c_void) as usize;
            early_println!(
                "{:4}: fn {:#18x} - pc {:#18x} / registers:",
                data.counter,
                fde_initial_address,
                pc,
            );
        }
        // Print the first 8 general registers for any architecture. The register number follows
        // the DWARF standard.
        for i in 0..8u16 {
            let reg_i = _Unwind_GetGR(unwind_ctx, i as i32);
            cfg_if::cfg_if! {
                if #[cfg(target_arch = "x86_64")] {
                    let reg_name = gimli::X86_64::register_name(Register(i)).unwrap_or("unknown");
                } else {
                    let reg_name = "unknown";
                }
            }
            if i % 4 == 0 {
                early_print!("\n    ");
            }
            early_print!(" {} {:#18x};", reg_name, reg_i);
        }
        early_print!("\n\n");
        UnwindReasonCode::NO_REASON
    }

    let mut data = CallbackData { counter: 0 };
    _Unwind_Backtrace(callback, &mut data as *mut _ as _);
}
