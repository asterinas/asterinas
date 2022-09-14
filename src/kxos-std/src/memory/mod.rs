pub mod elf;
pub mod init_stack;
pub mod vm_page;
pub mod aux_vec;
use alloc::ffi::CString;
use kxos_frame::{debug, vm::VmSpace};

use self::elf::{ElfError, ElfLoadInfo};

/// load elf to a given vm_space. this function will  
/// 1. read the vaddr of each segment to get all elf pages.  
/// 2. allocate physical frames and copy elf data to these frames
/// 3. map frames to the correct vaddr
/// 4. (allocate frams and) map the user stack
pub fn load_elf_to_vm_space<'a>(
    filename: CString,
    elf_file_content: &'a [u8],
    vm_space: &VmSpace,
) -> Result<ElfLoadInfo<'a>, ElfError> {
    let mut elf_load_info = ElfLoadInfo::parse_elf_data(elf_file_content, filename)?;
    elf_load_info.copy_data(vm_space)?;
    elf_load_info.debug_check_map_result(vm_space);
    debug!("map elf success");
    elf_load_info.init_stack(vm_space);
    Ok(elf_load_info)
}
