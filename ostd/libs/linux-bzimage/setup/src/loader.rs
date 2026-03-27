// SPDX-License-Identifier: MPL-2.0

use xmas_elf::program::{ProgramHeader, SegmentData};

#[cfg(all(feature = "cvm_guest", target_arch = "x86_64"))]
extern crate alloc;

#[cfg(all(feature = "cvm_guest", target_arch = "x86_64"))]
use alloc::vec::Vec;

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

/// Returns merged physical ranges of all PT_LOAD segments in the ELF file.
///
/// Each range is `[p_paddr, p_paddr + p_memsz)`, so `.bss` is included.
#[cfg(all(feature = "cvm_guest", target_arch = "x86_64"))]
pub(crate) fn elf_load_ranges(file: &[u8]) -> Vec<(u64, u64)> {
    let elf = xmas_elf::ElfFile::new(file).unwrap();
    let mut ranges = Vec::new();

    for ph in elf.program_iter() {
        if let ProgramHeader::Ph64(program) = ph {
            if program.get_type().unwrap() == xmas_elf::program::Type::Load && program.mem_size > 0
            {
                let start = program.physical_addr;
                let end = start
                    .checked_add(program.mem_size)
                    .expect("[setup] PT_LOAD physical range overflows.");
                ranges.push((start, end));
            }
        } else {
            panic!(
                "[setup] Unexpected program header type! Asterinas should be 64-bit ELF binary."
            );
        }
    }

    ranges.sort_unstable_by_key(|(start, _)| *start);

    let mut merged = Vec::new();
    for (start, end) in ranges {
        if let Some((_, last_end)) = merged.last_mut()
            && start <= *last_end
        {
            if end > *last_end {
                *last_end = end;
            }
            continue;
        }
        merged.push((start, end));
    }

    merged
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
    right.write_filled(0);
}
