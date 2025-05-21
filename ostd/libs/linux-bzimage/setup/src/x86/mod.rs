// SPDX-License-Identifier: MPL-2.0

use core::arch::global_asm;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        mod amd64_efi;

        pub use amd64_efi::alloc::alloc_at;

        const CFG_TARGET_ARCH_X86_64: usize = 1;
    } else if #[cfg(target_arch = "x86")] {
        mod legacy_i386;

        pub use legacy_i386::alloc::alloc_at;

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

global_asm!(
    ".section \".payload\", \"a\"",
    concat!(".incbin \"", env!("PAYLOAD_FILE"), "\""),
);

/// Returns an immutable slice containing the payload (i.e., the kernel).
fn payload() -> &'static [u8] {
    extern "C" {
        fn __payload_start();
        fn __payload_end();
    }

    // SAFETY: The memory region is part of the "rodata" segment, which is initialized, live for
    // `'static`, and never mutated.
    unsafe {
        core::slice::from_raw_parts(
            __payload_start as *const u8,
            __payload_end as usize - __payload_start as usize,
        )
    }
}
