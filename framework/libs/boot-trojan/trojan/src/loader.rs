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
            dst_slice[..program.file_size as usize].copy_from_slice(header_data);
            let zero_slice = &mut dst_slice[program.file_size as usize..];
            zero_slice.fill(0);
        }
    }

    // Return the Linux Boot Protocol entry point defined by Asterinas.
    crate::x86::ASTER_ENTRY_POINT
}
