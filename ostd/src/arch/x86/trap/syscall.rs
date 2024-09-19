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
// * Revise some comments.
//
// These changes are released under the following license:
//
// SPDX-License-Identifier: MPL-2.0

//! Configure fast syscall.

use core::arch::global_asm;

use x86::cpuid::CpuId;
use x86_64::{
    registers::{
        control::{Cr4, Cr4Flags},
        model_specific::{Efer, EferFlags, LStar, SFMask},
        rflags::RFlags,
    },
    VirtAddr,
};

use super::UserContext;

global_asm!(include_str!("syscall.S"));

pub fn init() {
    let cpuid = CpuId::new();
    unsafe {
        // Enable `syscall` instruction.
        assert!(cpuid
            .get_extended_processor_and_feature_identifiers()
            .unwrap()
            .has_syscall_sysret());
        Efer::update(|efer| {
            efer.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });

        // Enable `FSGSBASE` instructions.
        assert!(cpuid.get_extended_feature_info().unwrap().has_fsgsbase());
        Cr4::update(|cr4| {
            cr4.insert(Cr4Flags::FSGSBASE);
        });

        // Flags to clear on syscall.
        // Copy from Linux 5.0, TF|DF|IF|IOPL|AC|NT
        const RFLAGS_MASK: u64 = 0x47700;

        LStar::write(VirtAddr::new(syscall_entry as usize as u64));
        SFMask::write(RFlags::from_bits(RFLAGS_MASK).unwrap());
    }
}

extern "sysv64" {
    fn syscall_entry();
    fn syscall_return(regs: &mut UserContext);
}

impl UserContext {
    /// Go to user space with the context, and come back when a trap occurs.
    ///
    /// On return, the context will be reset to the status before the trap.
    /// Trap reason and error code will be placed at `trap_num` and `error_code`.
    ///
    /// If the trap was triggered by `syscall` instruction, the `trap_num` will be set to `0x100`.
    ///
    /// If `trap_num` is `0x100`, it will go user by `sysret` (`rcx` and `r11` are dropped),
    /// otherwise it will use `iret`.
    ///
    /// # Example
    /// ```no_run
    /// use trapframe::{UserContext, GeneralRegs};
    ///
    /// // init user space context
    /// let mut context = UserContext {
    ///     general: GeneralRegs {
    ///         rip: 0x1000,
    ///         rsp: 0x10000,
    ///         ..Default::default()
    ///     },
    ///     ..Default::default()
    /// };
    /// // go to user
    /// context.run();
    /// // back from user
    /// println!("back from user: {:#x?}", context);
    /// ```
    pub fn run(&mut self) {
        unsafe {
            syscall_return(self);
        }
    }
}
