//! Panic support.

use alloc::{boxed::Box, string::ToString};
use core::ffi::c_void;

use crate::arch::qemu::{exit_qemu, QemuExitCode};
use crate::{early_print, early_println};
use log::error;

extern crate cfg_if;
extern crate gimli;
use gimli::Register;

use unwinding::{
    abi::{
        UnwindContext, UnwindReasonCode, _Unwind_Backtrace, _Unwind_FindEnclosingFunction,
        _Unwind_GetGR, _Unwind_GetIP,
    },
    panic::begin_panic,
};

fn abort() -> ! {
    exit_qemu(QemuExitCode::Failed);
}

#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    let throw_info = ktest::PanicInfo {
        message: info.message().unwrap().to_string(),
        file: info.location().unwrap().file().to_string(),
        line: info.location().unwrap().line() as usize,
        col: info.location().unwrap().column() as usize,
    };
    // Throw an exception and expecting it to be caught.
    begin_panic(Box::new(throw_info.clone()));
    // If the exception is not caught (e.g. by ktest) and resumed,
    // then print the information and abort.
    error!("Uncaught panic!");
    early_println!("{}", info);
    early_println!("printing stack trace:");
    print_stack_trace();
    abort();
}

fn print_stack_trace() {
    struct CallbackData {
        counter: usize,
    }
    extern "C" fn callback(unwind_ctx: &UnwindContext<'_>, arg: *mut c_void) -> UnwindReasonCode {
        let data = unsafe { &mut *(arg as *mut CallbackData) };
        data.counter += 1;
        let pc = _Unwind_GetIP(unwind_ctx);
        let fde_initial_address = _Unwind_FindEnclosingFunction(pc as *mut c_void) as usize;
        early_println!(
            "{:4}: fn {:#18x} - pc {:#18x} / registers:",
            data.counter,
            fde_initial_address,
            pc,
        );
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
