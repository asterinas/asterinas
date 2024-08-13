// SPDX-License-Identifier: MPL-2.0

//! The standard library for Asterinas and other Rust OSes.
#![feature(alloc_error_handler)]
#![feature(const_mut_refs)]
#![feature(const_ptr_sub_ptr)]
#![feature(const_trait_impl)]
#![feature(core_intrinsics)]
#![feature(coroutines)]
#![feature(fn_traits)]
#![feature(generic_const_exprs)]
#![feature(iter_from_coroutine)]
#![feature(let_chains)]
#![feature(min_specialization)]
#![feature(negative_impls)]
#![feature(new_uninit)]
#![feature(panic_info_message)]
#![feature(ptr_sub_ptr)]
#![feature(strict_provenance)]
// The `generic_const_exprs` feature is incomplete however required for the page table
// const generic implementation. We are using this feature in a conservative manner.
#![allow(incomplete_features)]
#![allow(internal_features)]
#![no_std]
#![warn(missing_docs)]

extern crate alloc;
extern crate static_assertions;

pub mod arch;
pub mod boot;
pub mod bus;
pub mod collections;
pub mod console;
pub mod cpu;
mod error;
pub mod io_mem;
pub mod logger;
pub mod mm;
pub mod panicking;
pub mod prelude;
pub mod sync;
pub mod task;
pub mod trap;
pub mod user;

use arch::qemu::{exit_qemu, QemuExitCode};
pub use ostd_macros::{ktest, main};
pub use ostd_pod::Pod;
/// The module re-exports everything from the ktest crate
//
pub use ostd_test::*;

pub use self::{error::Error, prelude::Result};
// [`CpuLocalCell`] is easy to be mis-used, so we don't expose it to the users.
pub(crate) use crate::cpu::local::cpu_local_cell;

/// Initializes OSTD.
///
/// This function represents the first phase booting up the system. It makes
/// all functionalities of OSTD available after the call.
///
/// TODO: We need to refactor this function to make it more modular and
/// make inter-initialization-dependencies more clear and reduce usages of
/// boot stage only global variables.
pub fn init() {
    arch::enable_cpu_features();
    arch::serial::init();

    #[cfg(feature = "cvm_guest")]
    arch::check_tdx_init();

    // SAFETY: This function is called only once and only on the BSP.
    unsafe { cpu::local::early_init_bsp_local_base() };

    mm::heap_allocator::init();

    boot::init();
    logger::init();

    mm::page::allocator::init();
    mm::kspace::init_boot_page_table();
    mm::kspace::init_kernel_page_table(mm::init_page_meta());
    mm::misc_init();

    trap::init();
    arch::init_on_bsp();

    bus::init();

    mm::kspace::activate_kernel_page_table();

    invoke_ffi_init_funcs();
}

/// Invoke the initialization functions defined in the FFI.
/// The component system uses this function to call the initialization functions of
/// the components.
fn invoke_ffi_init_funcs() {
    extern "C" {
        fn __sinit_array();
        fn __einit_array();
    }
    let call_len = (__einit_array as usize - __sinit_array as usize) / 8;
    for i in 0..call_len {
        unsafe {
            let function = (__sinit_array as usize + 8 * i) as *const fn();
            (*function)();
        }
    }
}

/// The entry point for the kernel tests.
pub fn ktest_main() {
    unsafe {
        use crate::task::TaskOptions;

        extern "Rust" {
            pub static KTEST_TEST_WHITELIST: Option<&'static [&'static str]>;
            pub static KTEST_CRATE_WHITELIST: Option<&'static [&'static str]>;
        }
        let test_task = move || {
            run_ktests(KTEST_TEST_WHITELIST, KTEST_CRATE_WHITELIST);
        };
        let _ = TaskOptions::new(test_task).data(()).spawn();
    }
}

fn run_ktests(test_whitelist: Option<&[&str]>, crate_whitelist: Option<&[&str]>) -> ! {
    use alloc::{boxed::Box, string::ToString};
    use core::any::Any;

    use ostd_test::runner::{run_ktests, KtestResult};

    use crate::console::early_print;

    let fn_catch_unwind = &(unwinding::panic::catch_unwind::<(), fn()>
        as fn(fn()) -> core::result::Result<(), Box<(dyn Any + Send + 'static)>>);

    match run_ktests(
        &early_print,
        fn_catch_unwind,
        test_whitelist.map(|s| s.iter().map(|s| s.to_string())),
        crate_whitelist,
    ) {
        KtestResult::Ok => exit_qemu(QemuExitCode::Success),
        KtestResult::Failed => exit_qemu(QemuExitCode::Failed),
    };
}

/// Simple unit tests for the ktest framework.

mod test {
    use crate::prelude::*;

    #[ktest]
    fn trivial_assertion() {
        let _ = 1;
    }

    #[ktest]
    #[should_panic]
    fn failing_assertion() {
        assert_eq!(0, 1);
    }

    #[ktest]
    #[should_panic(expected = "expected panic message")]
    fn expect_panic() {
        panic!("expected panic message");
    }
}
