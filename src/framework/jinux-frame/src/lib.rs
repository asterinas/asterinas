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
#![feature(link_llvm_intrinsics)]
#![feature(strict_provenance)]
#![feature(const_trait_impl)]
#![feature(const_ops)]

extern crate alloc;

mod boot;
pub mod config;
pub mod cpu;
pub mod device;
mod driver;
mod error;
pub mod logger;
pub mod prelude;
pub mod sync;
pub mod task;
pub mod time;
pub mod timer;
pub mod trap;
pub mod user;
mod util;
pub mod vm;

pub use self::error::Error;
pub use self::prelude::Result;
use alloc::vec::Vec;
use core::{mem, panic::PanicInfo};
pub use limine::LimineModuleRequest;
use trap::{IrqCallbackHandle, IrqLine};
use trapframe::TrapFrame;
pub use util::AlignExt;

static mut IRQ_CALLBACK_LIST: Vec<IrqCallbackHandle> = Vec::new();

#[cfg(not(feature = "serial_print"))]
pub use crate::screen_print as print;
#[cfg(not(feature = "serial_print"))]
pub use crate::screen_println as println;

#[cfg(feature = "serial_print")]
pub use crate::console_print as print;
#[cfg(feature = "serial_print")]
pub use crate::console_println as println;

pub fn init() {
    device::serial::init();
    logger::init();
    boot::init();
    vm::init();
    trap::init();
    device::init();
    driver::init();
    enable_common_cpu_features();
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
        fn sinit_array();
        fn einit_array();
    }
    let call_len = (einit_array as u64 - sinit_array as u64) / 8;
    for i in 0..call_len {
        unsafe {
            let address = (sinit_array as u64 + 8 * i) as *const u64;
            let function = address as *const fn();
            (*function)();
        }
    }
}

fn enable_common_cpu_features() {
    use x86_64::registers::{control::Cr4Flags, model_specific::EferFlags, xcontrol::XCr0Flags};
    let mut cr4 = x86_64::registers::control::Cr4::read();
    cr4 |= Cr4Flags::FSGSBASE | Cr4Flags::OSXSAVE | Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE;
    unsafe {
        x86_64::registers::control::Cr4::write(cr4);
    }

    let mut xcr0 = x86_64::registers::xcontrol::XCr0::read();
    xcr0 |= XCr0Flags::AVX | XCr0Flags::SSE;
    unsafe {
        x86_64::registers::xcontrol::XCr0::write(xcr0);
    }

    unsafe {
        // enable non-executable page protection
        x86_64::registers::model_specific::Efer::update(|efer| {
            *efer |= EferFlags::NO_EXECUTE_ENABLE;
        });
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
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        console_print!("{}...\n", core::any::type_name::<T>());
        self();
        console_println!("[ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    console_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    console_println!("[failed]");
    console_println!("Error: {}", info);
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
    unreachable!()
}
