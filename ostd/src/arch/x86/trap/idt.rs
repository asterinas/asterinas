// SPDX-License-Identifier: MPL-2.0 OR MIT
//
// The original source code is from [trapframe-rs](https://github.com/rcore-os/trapframe-rs),
// which is released under the following license:
//
// SPDX-License-Identifier: MIT
//
// Copyright (c) 2020 - 2024 Runji Wang
//
// We make the following new changes:
// * Include `trap.S` in this file and remove unused function `sidt`.
// * Link `VECTORS` to `trap_handler_table` defined in `trap.S`.
//
// These changes are released under the following license:
//
// SPDX-License-Identifier: MPL-2.0

//! Configure Interrupt Descriptor Table (GDT).

use alloc::boxed::Box;
use core::arch::global_asm;

use x86_64::{
    structures::idt::{Entry, HandlerFunc, InterruptDescriptorTable},
    PrivilegeLevel,
};

global_asm!(include_str!("trap.S"));

pub fn init() {
    extern "C" {
        #[link_name = "trap_handler_table"]
        static VECTORS: [HandlerFunc; 256];
    }

    let idt = Box::leak(Box::new(InterruptDescriptorTable::new()));
    let entries: &'static mut [Entry<HandlerFunc>; 256] =
        unsafe { core::mem::transmute_copy(&idt) };
    for i in 0..256 {
        let opt = unsafe { entries[i].set_handler_fn(VECTORS[i]) };
        // Enable user space `int3` and `into`.
        if i == 3 || i == 4 {
            opt.set_privilege_level(PrivilegeLevel::Ring3);
        }
    }
    idt.load();
}
