//! The framework part of Jinux.
#![no_std]
#![allow(dead_code)]
#![allow(unused_variables)]
#![feature(negative_impls)]
#![feature(fn_traits)]
#![feature(const_maybe_uninit_zeroed)]
#![feature(alloc_error_handler)]
#![feature(core_intrinsics)]
#![feature(new_uninit)]
#![feature(strict_provenance)]
#![feature(link_llvm_intrinsics)]
#![feature(const_trait_impl)]
#![feature(generators)]
#![feature(iter_from_generator)]
#![feature(const_mut_refs)]
#![feature(custom_test_frameworks)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

pub mod arch;
pub mod boot;
pub mod bus;
pub mod config;
pub mod cpu;
mod error;
pub mod io_mem;
pub mod logger;
pub mod prelude;
pub mod sync;
pub mod task;
pub mod test;
pub mod timer;
pub mod trap;
pub mod user;
mod util;
pub mod vm;

pub use self::cpu::CpuLocal;
pub use self::error::Error;
pub use self::prelude::Result;
use alloc::vec::Vec;
use core::mem;
use trap::{IrqCallbackHandle, IrqLine};

static mut IRQ_CALLBACK_LIST: Vec<IrqCallbackHandle> = Vec::new();

pub fn init() {
    arch::before_all_init();
    logger::init();
    vm::heap_allocator::init();
    boot::init();
    vm::init();
    trap::init();
    arch::after_all_init();
    io_mem::init();
    bus::init();
    register_irq_common_callback();
    invoke_c_init_funcs();
}

fn register_irq_common_callback() {
    unsafe {
        for i in 0..256 {
            IRQ_CALLBACK_LIST.push(IrqLine::acquire(i as u8).on_active(
                |_trap_frame| {
                    todo!()
                    // info!("general handler");
                    // println!("{:#x?}", trap_frame);
                    // println!("rip = 0x{:x}", trap_frame.rip);
                    // println!("rsp = 0x{:x}", trap_frame.rsp);
                    // println!("cr2 = 0x{:x}", trap_frame.cr2);
                    // // println!("rbx = 0x{:x}", trap_frame.)
                    // panic!("couldn't handle trap right now");
                }
            ))
        }
    }
}

fn invoke_c_init_funcs() {
    extern "C" {
        fn __sinit_array();
        fn __einit_array();
    }
    let call_len = (__einit_array as u64 - __sinit_array as u64) / 8;
    for i in 0..call_len {
        unsafe {
            let address = (__sinit_array as u64 + 8 * i) as *const u64;
            let function = address as *const fn();
            (*function)();
        }
    }
}

#[inline(always)]
pub(crate) const fn zero<T>() -> T {
    unsafe { mem::MaybeUninit::zeroed().assume_init() }
}
