//! The framework part of KxOS.
#![no_std]
#![allow(dead_code)]
#![allow(unused_variables)]
#![feature(negative_impls)]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]
#![feature(fn_traits)]
#![feature(linked_list_cursors)]

extern crate alloc;

mod allocator;
pub mod config;
pub mod cpu;
pub mod device;
mod error;
pub mod prelude;
pub mod task;
pub mod timer;
pub mod user;
mod util;
pub mod log;
pub mod vm;
pub mod sync;

pub use self::error::Error;
use alloc::sync::Arc;
use bootloader::BootInfo;
use device::{InterruptInformation, IrqCallbackHandle, IrqLine};

static mut STORE: Option<IrqCallbackHandle> = None;

pub fn init(boot_info: &'static mut BootInfo) {
    allocator::init();
    device::init(boot_info);
    device::framebuffer::WRITER.lock().as_mut().unwrap().clear();
    // breakpoint
    let breakpoint_irq: Arc<&IrqLine>;
    unsafe {
        breakpoint_irq = IrqLine::acquire(3);
    }
    let a = breakpoint_irq.on_active(breakpoint_handler);
    let b = breakpoint_irq.on_active(breakpoint_handler);
    unsafe {
        STORE = Some(a);
    }
    x86_64::instructions::interrupts::int3(); // new
}

fn breakpoint_handler(interrupt_information: InterruptInformation) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", interrupt_information);
}
