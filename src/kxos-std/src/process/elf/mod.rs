pub mod aux_vec;
pub mod elf;
pub mod init_stack;

use kxos_frame::vm::VmSpace;

use self::elf::ElfLoadInfo;
use crate::prelude::*;

/// load elf to a given vm_space. this function will  
/// 1. read the vaddr of each segment to get all elf pages.  
/// 2. allocate physical frames and copy elf data to these frames
/// 3. map frames to the correct vaddr
/// 4. (allocate frams and) map the user stack
pub fn load_elf_to_vm_space<'a>(
    filename: CString,
    elf_file_content: &'a [u8],
    vm_space: &VmSpace,
) -> Result<ElfLoadInfo<'a>> {
    let mut elf_load_info = ElfLoadInfo::parse_elf_data(elf_file_content, filename)?;
    elf_load_info.copy_and_map_segments(vm_space)?;
    elf_load_info.debug_check_map_result(vm_space);
    elf_load_info.init_stack(vm_space);
    elf_load_info.write_program_header_table(vm_space, elf_file_content);
    debug!("load elf succeeds.");

    Ok(elf_load_info)
}
