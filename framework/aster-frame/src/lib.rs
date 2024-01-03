// SPDX-License-Identifier: MPL-2.0

//! The framework part of Asterinas.
#![feature(alloc_error_handler)]
#![feature(const_mut_refs)]
#![feature(const_ptr_sub_ptr)]
#![feature(const_trait_impl)]
#![feature(coroutines)]
#![feature(fn_traits)]
#![feature(iter_from_coroutine)]
#![feature(let_chains)]
#![feature(negative_impls)]
#![feature(new_uninit)]
#![feature(panic_info_message)]
#![feature(ptr_sub_ptr)]
#![feature(strict_provenance)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![no_std]

extern crate alloc;
#[macro_use]
extern crate ktest;
#[macro_use]
extern crate static_assertions;

pub mod arch;
pub mod boot;
pub mod bus;
pub mod config;
pub mod console;
pub mod cpu;
mod error;
pub mod io_mem;
pub mod logger;
pub mod panicking;
pub mod prelude;
pub mod sync;
pub mod task;
pub mod timer;
pub mod trap;
pub mod user;
mod util;
pub mod vm;

pub use self::cpu::CpuLocal;
pub use self::error::Error;
pub use self::prelude::Result;
#[cfg(feature = "intel_tdx")]
use tdx_guest::init_tdx;

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
    vm::heap_allocator::init();
    boot::init();
    vm::init();
    trap::init();
    arch::after_all_init();
    bus::init();
    invoke_ffi_init_funcs();
}

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
#[if_cfg_ktest]
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
