// SPDX-License-Identifier: MPL-2.0

//! Panic support.

use core::ffi::c_void;

use crate::{
    arch::qemu::{exit_qemu, QemuExitCode},
    early_print, early_println,
    sync::SpinLock,
};

extern crate cfg_if;
extern crate gimli;

use gimli::Register;

/// The default panic handler for OSTD based kernels.
///
/// The user can override it by defining their own panic handler with the macro
/// `#[ostd::panic_handler]`.
#[linkage = "weak"]
#[no_mangle]
pub fn __ostd_panic_handler(info: &core::panic::PanicInfo) -> ! {
    let _irq_guard = crate::trap::irq::disable_local();

    crate::cpu_local_cell! {
        static IN_PANIC: bool = false;
    }

    if IN_PANIC.load() {
        early_println!("The panic handler panicked {:#?}", info);
        abort();
    }

    IN_PANIC.store(true);

    early_println!("Non-resettable panic! {:#?}", info);

    print_stack_trace();
    abort();
}

/// Aborts the QEMU
pub fn abort() -> ! {
    exit_qemu(QemuExitCode::Failed);
}

#[cfg(not(target_arch = "loongarch64"))]
pub use unwinding::panic::{begin_panic, catch_unwind};

/// Prints the stack trace of the current thread to the console.
///
/// The printing procedure is protected by a spin lock to prevent interleaving.
#[cfg(not(target_arch = "loongarch64"))]
pub fn print_stack_trace() {
    use unwinding::abi::{
        UnwindContext, UnwindReasonCode, _Unwind_Backtrace, _Unwind_FindEnclosingFunction,
        _Unwind_GetGR, _Unwind_GetIP,
    };

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
                } else if #[cfg(target_arch = "riscv64")] {
                    let reg_name = gimli::RiscV::register_name(Register(i)).unwrap_or("unknown");
                } else if #[cfg(target_arch = "aarch64")] {
                    let reg_name = gimli::AArch64::register_name(Register(i)).unwrap_or("unknown");
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

#[cfg(target_arch = "loongarch64")]
pub fn catch_unwind<R, F: FnOnce() -> R>(
    f: F,
) -> Result<R, alloc::boxed::Box<dyn core::any::Any + Send>> {
    // TODO: Support unwinding in LoongArch.
    Ok(f())
}

#[cfg(target_arch = "loongarch64")]
pub fn begin_panic<R>(_: alloc::boxed::Box<R>) {
    // TODO: Support panic context in LoongArch.
}

#[cfg(target_arch = "loongarch64")]
pub fn print_stack_trace() {
    // TODO: Support stack trace print in LoongArch.
    early_println!("Printing stack trace:");
}
