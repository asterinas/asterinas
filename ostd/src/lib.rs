// SPDX-License-Identifier: MPL-2.0

//! The standard library for Asterinas and other Rust OSes.
#![feature(alloc_error_handler)]
#![feature(allocator_api)]
#![feature(btree_cursors)]
#![feature(const_ptr_sub_ptr)]
#![feature(const_trait_impl)]
#![feature(core_intrinsics)]
#![feature(coroutines)]
#![feature(fn_traits)]
#![feature(generic_const_exprs)]
#![feature(iter_from_coroutine)]
#![feature(let_chains)]
#![feature(linkage)]
#![feature(min_specialization)]
#![feature(negative_impls)]
#![feature(ptr_sub_ptr)]
#![feature(sync_unsafe_cell)]
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
pub mod panic;
pub mod prelude;
pub mod smp;
pub mod sync;
pub mod task;
pub mod timer;
pub mod trap;
pub mod user;

use core::sync::atomic::{AtomicBool, Ordering};

pub use ostd_macros::{main, panic_handler};
pub use ostd_pod::Pod;

pub use self::{error::Error, prelude::Result};

/// Initializes OSTD.
///
/// This function represents the first phase booting up the system. It makes
/// all functionalities of OSTD available after the call.
///
/// # Safety
///
/// This function should be called only once and only on the BSP.
//
// TODO: We need to refactor this function to make it more modular and
// make inter-initialization-dependencies more clear and reduce usages of
// boot stage only global variables.
#[doc(hidden)]
unsafe fn init() {
    arch::enable_cpu_features();
    arch::serial::init();

    #[cfg(feature = "cvm_guest")]
    arch::init_cvm_guest();

    // SAFETY: This function is called only once and only on the BSP.
    unsafe { cpu::local::early_init_bsp_local_base() };

    // SAFETY: This function is called only once and only on the BSP.
    unsafe { mm::heap_allocator::init() };

    boot::init();
    logger::init();

    mm::page::allocator::init();
    mm::kspace::init_kernel_page_table(mm::init_page_meta());
    mm::dma::init();

    arch::init_on_bsp();

    smp::init();

    // SAFETY: This function is called only once on the BSP.
    unsafe {
        mm::kspace::activate_kernel_page_table();
    }

    bus::init();

    arch::irq::enable_local();

    invoke_ffi_init_funcs();

    IN_BOOTSTRAP_CONTEXT.store(false, Ordering::Relaxed);
}

/// Indicates whether the kernel is in bootstrap context.
pub(crate) static IN_BOOTSTRAP_CONTEXT: AtomicBool = AtomicBool::new(true);

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
    use crate::prelude::*;

    #[ktest]
    #[allow(clippy::eq_op)]
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

#[doc(hidden)]
pub mod ktest {
    //! The module re-exports everything from the [`ostd_test`] crate, as well
    //! as the test entry point macro.
    //!
    //! It is rather discouraged to use the definitions here directly. The
    //! `ktest` attribute is sufficient for all normal use cases.

    pub use ostd_macros::{test_main as main, test_panic_handler as panic_handler};
    pub use ostd_test::*;
}
