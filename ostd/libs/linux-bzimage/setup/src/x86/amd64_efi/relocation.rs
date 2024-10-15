// SPDX-License-Identifier: MPL-2.0

use crate::x86::get_image_loaded_offset;

/// Apply the relocations in the `.rela.*` sections.
///
/// The function will enable dyn Trait objects to work since they rely on
/// vtable pointers. Vtable won't work without relocations.
///
/// We currently support R_X86_64_RELATIVE relocations only. And this type of
/// relocation seems to be the only existing type if we compile Rust code to
/// PIE ELF binaries.
///
/// # Safety
///
/// This function will modify the memory pointed by the relocations. And the
/// Rust memory safety mechanisms are not aware of these kind of modification.
/// Failure to do relocations will cause `dyn Trait` objects to break.
pub unsafe fn apply_rela_relocations() {
    use core::arch::asm;
    let image_loaded_offset = get_image_loaded_offset();

    let mut start: usize;
    let end: usize;

    unsafe {
        asm!(
            "lea {}, [rip + __rela_start]",
            out(reg) start,
        );
        asm!(
            "lea {}, [rip + __rela_end]",
            out(reg) end,
        );
    }

    #[cfg(feature = "debug_print")]
    unsafe {
        use crate::console::{print_hex, print_str};
        print_str("[EFI stub debug] loaded offset = ");
        print_hex(image_loaded_offset as u64);
        print_str("\n");
        print_str("[EFI stub debug] .rela section start = ");
        print_hex(start as u64);
        print_str(", end = ");
        print_hex(end as u64);
        print_str("\n");
    }

    #[cfg(feature = "debug_print")]
    let mut count = 0;

    while start < end {
        let rela = (start as *const Elf64Rela).read_volatile();
        let r_type = (rela.r_info & 0xffffffff) as u32;
        let _r_sym = (rela.r_info >> 32) as usize;
        let r_addend = rela.r_addend as isize;
        let r_offset = rela.r_offset as isize;
        let target = image_loaded_offset.wrapping_add(r_offset) as usize;
        #[cfg(feature = "debug_print")]
        unsafe {
            use crate::console::{print_hex, print_str};
            count += 1;
            print_str("[EFI stub debug] Applying relocation #");
            print_hex(count as u64);
            print_str(" at offset ");
            print_hex(r_offset as u64);
            print_str(", type = ");
            print_hex(r_type as u64);
            print_str(", addend = ");
            print_hex(r_addend as u64);
            print_str("\n");
        }
        match r_type {
            R_X86_64_RELATIVE => {
                let value = image_loaded_offset.wrapping_add(r_addend) as usize;
                (target as *mut usize).write(value);
            }
            _ => {
                panic!("Unknown relocation type: {}", r_type);
            }
        }
        start = start.wrapping_add(core::mem::size_of::<Elf64Rela>());
    }
}

const R_X86_64_RELATIVE: u32 = 8;

#[derive(Copy, Clone)]
#[repr(C)]
struct Elf64Rela {
    r_offset: u64,
    r_info: u64,
    r_addend: i64,
}
