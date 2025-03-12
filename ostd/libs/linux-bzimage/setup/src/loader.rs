// SPDX-License-Identifier: MPL-2.0

use core::mem::MaybeUninit;

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
    let SegmentData::Undefined(segment_data) = program.get_data(file).unwrap() else {
        panic!("[setup] Unexpected segment data type!");
    };

    let dst_slice = crate::x86::alloc_at(program.physical_addr as usize, program.mem_size as usize);

    #[cfg(feature = "debug_print")]
    crate::println!(
        "[setup] Loading an ELF segment: addr={:#x}, size={:#x}",
        program.physical_addr,
        program.mem_size,
    );

    let (left, right) = dst_slice.split_at_mut(program.file_size as usize);
    left.write_copy_of_slice(segment_data);
    MaybeUninit::fill(right, 0);
}
