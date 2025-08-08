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

use super::RawUserContext;
use crate::mm::PagingConstsTrait;

global_asm!(
    include_str!("syscall.S"),
    USER_CS = const super::gdt::USER_CS.0,
    USER_SS = const super::gdt::USER_SS.0,
    ADDRESS_WIDTH = const crate::mm::kspace::KernelPtConfig::ADDRESS_WIDTH,
);

/// # Safety
///
/// The caller needs to ensure that `gdt::init` has been called before, so the segment selectors
/// used in the `syscall` and `sysret` instructions have been properly initialized.
pub(super) unsafe fn init() {
    let cpuid = CpuId::new();

    assert!(cpuid
        .get_extended_processor_and_feature_identifiers()
        .unwrap()
        .has_syscall_sysret());
    assert!(cpuid.get_extended_feature_info().unwrap().has_fsgsbase());

    // Flags to clear on syscall.
    //
    // Linux 5.0 uses TF|DF|IF|IOPL|AC|NT. Reference:
    // <https://github.com/torvalds/linux/blob/v5.0/arch/x86/kernel/cpu/common.c#L1559-L1562>
    const RFLAGS_MASK: u64 = 0x47700;

    // SAFETY: The segment selectors are correctly initialized (as upheld by the caller), and the
    // entry point and flags to clear are also correctly set, so enabling the `syscall` and
    // `sysret` instructions is safe.
    unsafe {
        LStar::write(VirtAddr::new(syscall_entry as usize as u64));
        SFMask::write(RFlags::from_bits(RFLAGS_MASK).unwrap());

        // Enable the `syscall` and `sysret` instructions.
        Efer::update(|efer| {
            efer.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });
    }

    // SAFETY: Enabling the `rdfsbase`, `wrfsbase`, `rdgsbase`, and `wrgsbase` instructions is safe
    // as long as the kernel properly deals with the arbitrary base values set by the userspace
    // program. (FIXME: Do we really need to unconditionally enable them?)
    unsafe {
        Cr4::update(|cr4| {
            cr4.insert(Cr4Flags::FSGSBASE);
        })
    };
}

extern "sysv64" {
    fn syscall_entry();
    fn syscall_return(regs: &mut RawUserContext);
}

impl RawUserContext {
    /// Goes to user space with the context, and comes back when a trap occurs.
    ///
    /// On return, the context will be reset to the status before the trap.
    /// Trap reason and error code will be placed at `trap_num` and `error_code`.
    ///
    /// If the trap was triggered by `syscall` instruction, the `trap_num` will be set to `0x100`.
    ///
    /// If `trap_num` is `0x100`, it will go user by `sysret` (`rcx` and `r11` are dropped),
    /// otherwise it will use `iret`.
    pub(in crate::arch) fn run(&mut self) {
        // Return to userspace with interrupts disabled. Otherwise, interrupts
        // after executing `swapgs` will mess up the CPU state.
        crate::arch::irq::disable_local();
        unsafe {
            syscall_return(self);
        }
    }
}
