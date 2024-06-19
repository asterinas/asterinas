// SPDX-License-Identifier: MPL-2.0

use xmas_elf::program::{ProgramHeader, SegmentData};

/// Load the kernel ELF payload to memory.
pub fn load_elf(file: &[u8]) {
    let elf = xmas_elf::ElfFile::new(file).unwrap();

    for ph in elf.program_iter() {
        let ProgramHeader::Ph64(program) = ph else {
            panic!(
                "[setup] Unexpected program header type! Asterinas should be 64-bit ELF binary."
            );
        };
        if program.get_type().unwrap() == xmas_elf::program::Type::Load {
            load_segment(&elf, program);
        }
    }
}

fn load_segment(file: &xmas_elf::ElfFile, program: &xmas_elf::program::ProgramHeader64) {
    let SegmentData::Undefined(header_data) = program.get_data(file).unwrap() else {
        panic!("[setup] Unexpected segment data type!");
    };
    // SAFETY: the physical address from the ELF file is valid
    let dst_slice = unsafe {
        core::slice::from_raw_parts_mut(program.physical_addr as *mut u8, program.mem_size as usize)
    };
    /* crate::println!(
        "[setup loader debug] loading ELF segment at {:#x}, size = {:#x}",
        program.physical_addr,
        program.mem_size,
    ); */
    #[cfg(feature = "debug_print")]
    unsafe {
        use crate::console::{print_hex, print_str};
        print_str("[setup loader debug] loading ELF segment at ");
        print_hex(program.physical_addr as u64);
        print_str(", size = ");
        print_hex(program.mem_size as u64);
        print_str("\n");
    }
    // SAFETY: the ELF file is valid
    // dst_slice[..program.file_size as usize].copy_from_slice(header_data);
    unsafe {
        memcpy(
            dst_slice.as_mut_ptr(),
            header_data.as_ptr(),
            program.file_size as usize,
        );
    }
    let zero_slice = &mut dst_slice[program.file_size as usize..];
    zero_slice.fill(0);
}

/// TODO: remove this and use copy_from_slice instead
///
/// We use a custom memcpy because the standard library's compiler's builtin memcpy
/// fails for some unknown reason. Sometimes that will result in "Unknown OPCode"
/// machine error.
unsafe fn memcpy(dst: *mut u8, src: *const u8, size: usize) {
    let mut i = 0;
    while i < size {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
}
