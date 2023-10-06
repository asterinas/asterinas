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
//#![feature(link_llvm_intrinsics)]
#![feature(const_trait_impl)]
#![feature(generators)]
#![feature(iter_from_generator)]
#![feature(const_mut_refs)]
#![feature(let_chains)]

extern crate alloc;
#[macro_use]
extern crate static_assertions;

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
pub mod timer;
pub mod trap;
pub mod user;
mod util;
pub mod vm;

pub use self::cpu::CpuLocal;
pub use self::error::Error;
pub use self::prelude::Result;
use alloc::vec::Vec;
use arch::irq::{IrqCallbackHandle, IrqLine};
use core::{mem, panic::PanicInfo};
#[cfg(feature = "intel_tdx")]
use tdx_guest::init_tdx;
use trapframe::TrapFrame;

static mut IRQ_CALLBACK_LIST: Vec<IrqCallbackHandle> = Vec::new();

pub fn init() {
    arch::before_all_init();
    logger::init();
    #[cfg(feature = "intel_tdx")]
    let td_info = init_tdx().unwrap();
    #[cfg(feature = "intel_tdx")]
    println!(
        "td gpaw: {}, td attributes: {:?}\nTDX guest is initialized",
        td_info.gpaw, td_info.attributes
    );
    vm::heap_allocator::init();
    boot::init();
    vm::init();
    trap::init();
    arch::after_all_init();
    bus::init();
    register_irq_common_callback();
    invoke_c_init_funcs();
}

fn register_irq_common_callback() {
    unsafe {
        for i in 0..256 {
            IRQ_CALLBACK_LIST.push(IrqLine::acquire(i as u8).on_active(general_handler))
        }
    }
}

fn invoke_c_init_funcs() {
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

fn general_handler(trap_frame: &TrapFrame) {
    // info!("general handler");
    // println!("{:#x?}", trap_frame);
    // println!("rip = 0x{:x}", trap_frame.rip);
    // println!("rsp = 0x{:x}", trap_frame.rsp);
    // println!("cr2 = 0x{:x}", trap_frame.cr2);
    // // println!("rbx = 0x{:x}", trap_frame.)
    // panic!("couldn't handler trap right now");
}

#[inline(always)]
pub(crate) const fn zero<T>() -> T {
    unsafe { mem::MaybeUninit::zeroed().assume_init() }
}

pub trait Testable {
    fn run(&self);
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        print!("{}...\n", core::any::type_name::<T>());
        self();
        println!("[ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    println!("[failed]");
    println!("Error: {}", info);
    exit_qemu(QemuExitCode::Failed);
}

pub fn panic_handler() {
    // let mut fp: usize;
    // let stop = unsafe{
    //     Task::current().kstack.get_top()
    // };
    // info!("stop:{:x}",stop);
    // unsafe{
    //     asm!("mov rbp, {}", out(reg) fp);
    //     info!("fp:{:x}",fp);
    //     println!("---START BACKTRACE---");
    //     for i in 0..10 {
    //         if fp == stop {
    //             break;
    //         }
    //         println!("#{}:ra={:#x}", i, *((fp - 8) as *const usize));
    //         info!("fp target:{:x}",*((fp ) as *const usize));
    //         fp = *((fp - 16) as *const usize);
    //     }
    //     println!("---END   BACKTRACE---");
    // }
}

/// The exit code of x86 QEMU isa debug device. In `qemu-system-x86_64` the
/// exit code will be `(code << 1) | 1`. So you could never let QEMU invoke
/// `exit(0)`. We also need to check if the exit code is returned by the
/// kernel, so we couldn't use 0 as exit_success because this may conflict
/// with QEMU return value 1, which indicates that QEMU itself fails.
#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x20,
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
    unreachable!()
}
