pub mod elf;
pub mod user_stack;
pub mod vm_page;
use kxos_frame::vm::VmSpace;

use self::elf::{ElfError, ElfLoadInfo};

/// load elf to a given vm_space. this function will  
/// 1. read the vaddr of each segment to get all elf pages.  
/// 2. allocate physical frames and copy elf data to these frames
/// 3. map frames to the correct vaddr
/// 4. (allocate frams and) map the user stack
pub fn load_elf_to_vm_space<'a>(
    elf_file_content: &'a [u8],
    vm_space: &VmSpace,
) -> Result<ElfLoadInfo<'a>, ElfError> {
    let elf_load_info = ElfLoadInfo::parse_elf_data(elf_file_content)?;
    let to_frames = elf_load_info.copy_elf()?;
    elf_load_info.map_self(vm_space, to_frames)?;
    elf_load_info.map_and_clear_user_stack(vm_space);
    Ok(elf_load_info)
}
