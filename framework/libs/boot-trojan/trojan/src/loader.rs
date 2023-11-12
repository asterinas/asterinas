use xmas_elf::program::{ProgramHeader, SegmentData};

pub fn load_elf(file: &[u8]) -> u32 {
    let elf = xmas_elf::ElfFile::new(file).unwrap();

    for ph in elf.program_iter() {
        let ProgramHeader::Ph64(program) = ph else {
            panic!("[setup] Unexpected program header type!");
        };
        if program.get_type().unwrap() == xmas_elf::program::Type::Load {
            let SegmentData::Undefined(header_data) = program.get_data(&elf).unwrap() else {
                panic!("[setup] Unexpected segment data type!");
            };
            // Safety: the physical address from the ELF file is valid
            let dst_slice = unsafe {
                core::slice::from_raw_parts_mut(
                    program.physical_addr as *mut u8,
                    program.mem_size as usize,
                )
            };
            dst_slice[..program.file_size as usize].copy_from_slice(header_data);
            let zero_slice = &mut dst_slice[program.file_size as usize..];
            zero_slice.fill(0);
        }
    }

    // Return the Linux 32-bit Boot Protocol entry point defined by Asterinas.
    0x8001000
}
