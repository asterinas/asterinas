// SPDX-License-Identifier: MPL-2.0

use core::arch::global_asm;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        mod amd64_efi;

        const CFG_TARGET_ARCH_X86_64: usize = 1;
    } else if #[cfg(target_arch = "x86")] {
        mod legacy_i386;

        const CFG_TARGET_ARCH_X86_64: usize = 0;
    } else {
        compile_error!("unsupported target architecture");
    }
}

global_asm!(
    include_str!("header.S"),
    CFG_TARGET_ARCH_X86_64 = const CFG_TARGET_ARCH_X86_64,
);

/// Returns the difference between the real load address and the one in the linker script.
pub fn image_load_offset() -> isize {
    /// The load address of the `entry_legacy32` symbol specified in the linker script.
    const CODE32_START: isize = 0x100000;

    extern "C" {
        fn entry_legacy32();
    }

    (entry_legacy32 as usize as isize) - CODE32_START
}
