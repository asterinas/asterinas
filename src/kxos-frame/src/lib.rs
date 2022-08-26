//! The framework part of KxOS.
#![no_std]
#![allow(dead_code)]
#![allow(unused_variables)]
#![feature(negative_impls)]
#![feature(abi_x86_interrupt)]
#![feature(fn_traits)]
#![feature(linked_list_cursors)]
#![feature(const_maybe_uninit_zeroed)]
#![feature(alloc_error_handler)]
#![feature(core_intrinsics)]
#![feature(new_uninit)]
extern crate alloc;

pub mod cell;
pub mod config;
pub mod cpu;
pub mod device;
mod error;
pub mod log;
pub mod mm;
pub mod prelude;
pub mod sync;
pub mod task;
pub mod timer;
pub mod trap;
pub mod user;
mod util;
pub mod vm;
pub mod x86_64_util;

use core::mem;

pub use self::error::Error;
pub use self::sync::up::UPSafeCell;
use alloc::sync::Arc;
use bootloader::{
    boot_info::{FrameBuffer, MemoryRegionKind},
    BootInfo,
};
use trap::{IrqLine, TrapFrame};

pub fn init(boot_info: &'static mut BootInfo) {
    let siz = boot_info.framebuffer.as_ref().unwrap() as *const FrameBuffer as usize;
    device::init(boot_info.framebuffer.as_mut().unwrap());
    device::framebuffer::WRITER.lock().as_mut().unwrap().clear();
    println!("{:x}", siz);
    trap::init();
    let mut memory_init = false;
    // memory
    for region in boot_info.memory_regions.iter() {
        if region.kind == MemoryRegionKind::Usable {
            let start: u64 = region.start;
            let size: u64 = region.end - region.start;
            println!(
                "[kernel] physical frames start = {:x}, size = {:x}",
                start, size
            );
            mm::init(start, size);
            memory_init = true;
            break;
        }
    }
    if !memory_init {
        panic!("memory init failed");
    }

    // breakpoint
    let breakpoint_irq: Arc<&IrqLine>;
    unsafe {
        breakpoint_irq = IrqLine::acquire(3);
    }
    let a = breakpoint_irq.on_active(breakpoint_handler);
    x86_64::instructions::interrupts::int3(); // breakpoint
}

fn breakpoint_handler(interrupt_information: TrapFrame) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", interrupt_information);
}

#[inline(always)]
pub const fn zero<T>() -> T {
    unsafe { mem::MaybeUninit::zeroed().assume_init() }
}
