// SPDX-License-Identifier: MPL-2.0

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        mod amd64_efi;
    } else if #[cfg(target_arch = "x86")] {
        mod legacy_i386;
    } else {
        compile_error!("Unsupported target_arch");
    }
}

// This is enforced in the linker script of the setup.
const START_OF_SETUP32_VA: usize = 0x100000;

/// The setup is a position-independent executable. We can get the loaded base
/// address from the symbol.
#[inline]
pub fn get_image_loaded_offset() -> isize {
    let address_of_start: usize;
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "lea {}, [rip + start_of_setup32]",
            out(reg) address_of_start,
            options(pure, nomem, nostack)
        );
    }
    #[cfg(target_arch = "x86")]
    unsafe {
        core::arch::asm!(
            "lea {}, [start_of_setup32]",
            out(reg) address_of_start,
            options(pure, nomem, nostack)
        );
    }
    address_of_start as isize - START_OF_SETUP32_VA as isize
}
