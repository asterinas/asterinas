// SPDX-License-Identifier: MPL-2.0

use crate::x86::get_image_loaded_offset;

struct Elf64Rela {
    r_offset: u64,
    r_info: u64,
    r_addend: i64,
}

fn get_rela_array() -> &'static [Elf64Rela] {
    extern "C" {
        fn __rela_dyn_start();
        fn __rela_dyn_end();
    }
    let start = __rela_dyn_start as *const Elf64Rela;
    let end = __rela_dyn_end as *const Elf64Rela;
    // FIXME: 2023/11/29
    // There should be a Rust compiler bug that makes the calculation of len incorrect.
    // The most sound implementation only works in debug mode.
    // let len = unsafe { end.offset_from(start) } as usize;
    // The inline asm solution is a workaround.
    let len = unsafe {
        let len: usize;
        core::arch::asm!("
                mov {len}, {end}
                sub {len}, {start}
            ",
            len = out(reg) len,
            end = in(reg) end,
            start = in(reg) start,
        );
        len / core::mem::size_of::<Elf64Rela>() as usize
    };
    #[cfg(feature = "debug_print")]
    unsafe {
        use crate::console::{print_hex, print_str};
        print_str("[EFI stub debug] .rela.dyn section size = ");
        print_hex(len as u64);
        print_str("; __rela_dyn_start = ");
        print_hex(start as u64);
        print_str(", __rela_dyn_end = ");
        print_hex(end as u64);
        print_str("\n");
    }
    // Safety: the linker will ensure that the symbols are valid.
    unsafe { core::slice::from_raw_parts(start as *const Elf64Rela, len) }
}

const R_X86_64_RELATIVE: u32 = 8;

/// Apply the relocations in the `.rela.dyn` section.
///
/// The function will enable dyn Trait objects to work since they rely on vtable pointers. Vtable
/// won't work without relocations.
///
/// We currently support R_X86_64_RELATIVE relocations only. And this type of relocation seems to
/// be the only existing type if we compile Rust code to PIC ELF binaries.
///
/// # Safety
/// This function will modify the memory pointed by the relocations. And the Rust memory safety
/// mechanisms are not aware of these kind of modification. Failure to do relocations will cause
/// dyn Trait objects to break.
pub unsafe fn apply_rela_dyn_relocations() {
    let image_loaded_offset = get_image_loaded_offset();
    let relas = get_rela_array();
    for rela in relas {
        let r_type = (rela.r_info & 0xffffffff) as u32;
        let _r_sym = (rela.r_info >> 32) as usize;
        let r_addend = rela.r_addend;
        let r_offset = rela.r_offset as usize;
        let target = (image_loaded_offset + r_offset as isize) as usize;
        #[cfg(feature = "debug_print")]
        unsafe {
            use crate::console::{print_hex, print_str};
            print_str("[EFI stub debug] Applying relocation at offset ");
            print_hex(r_offset as u64);
            print_str(", type = ");
            print_hex(r_type as u64);
            print_str(", addend = ");
            print_hex(r_addend as u64);
            print_str("\n");
        }
        match r_type {
            R_X86_64_RELATIVE => {
                let value = (image_loaded_offset as i64 + r_addend) as usize;
                *(target as *mut usize) = value;
            }
            _ => {
                panic!("Unknown relocation type: {}", r_type);
            }
        }
    }
}
