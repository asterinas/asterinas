// SPDX-License-Identifier: MPL-2.0

//! The framework part of Asterinas.
#![feature(alloc_error_handler)]
#![feature(const_mut_refs)]
#![feature(const_ptr_sub_ptr)]
#![feature(const_trait_impl)]
#![feature(coroutines)]
#![feature(fn_traits)]
#![feature(generic_const_exprs)]
#![feature(iter_from_coroutine)]
#![feature(let_chains)]
#![feature(negative_impls)]
#![feature(new_uninit)]
#![feature(panic_info_message)]
#![feature(ptr_sub_ptr)]
#![feature(strict_provenance)]
#![feature(pointer_is_aligned)]
// The `generic_const_exprs` feature is incomplete however required for the page table
// const generic implementation. We are using this feature in a conservative manner.
#![allow(incomplete_features)]
#![no_std]
#![warn(missing_docs)]

extern crate alloc;
#[cfg(ktest)]
#[macro_use]
extern crate ktest;
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

#[cfg(feature = "intel_tdx")]
use tdx_guest::init_tdx;

pub use self::{cpu::CpuLocal, error::Error, prelude::Result};

/// Initialize the framework.
///
/// This function represents the first phase booting up the system. It makes
/// all functionalities of the framework available after the call.
///
/// TODO: We need to refactor this function to make it more modular and
/// make inter-initialization-dependencies more clear and reduce usages of
/// boot stage only global variables.
pub fn init() {
    arch::before_all_init();
    logger::init();

    #[cfg(feature = "intel_tdx")]
    let td_info = init_tdx().unwrap();
    #[cfg(feature = "intel_tdx")]
    early_println!(
        "td gpaw: {}, td attributes: {:?}\nTDX guest is initialized",
        td_info.gpaw,
        td_info.attributes
    );

    mm::heap_allocator::init();

    boot::init();

    mm::page::allocator::init();
    let mut boot_pt = mm::get_boot_pt();
    mm::kspace::init_kernel_page_table(mm::init_page_meta(&mut boot_pt));
    mm::misc_init();

    trap::init();
    arch::after_all_init();
    bus::init();

    mm::kspace::activate_kernel_page_table(boot_pt);

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

/// Simple unit tests for the ktest framework.
#[cfg(ktest)]
mod test {
    #[ktest]
    fn trivial_assertion() {
        assert_eq!(0, 0);
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
