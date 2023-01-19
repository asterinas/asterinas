pub mod aux_vec;
pub mod elf_file;
pub mod elf_segment_pager;
pub mod init_stack;
pub mod load_elf;

use self::load_elf::ElfLoadInfo;
use crate::{prelude::*, rights::Full, vm::vmar::Vmar};

/// load elf to the root vmar. this function will  
/// 1. read the vaddr of each segment to get all elf pages.  
/// 2. create a vmo for each elf segment, create a backup pager for each segment. Then map the vmo to the root vmar.
/// 3. write proper content to the init stack.
pub fn load_elf_to_root_vmar(
    elf_file_content: &'static [u8],
    root_vmar: &Vmar<Full>,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<ElfLoadInfo> {
    let mut elf_load_info = ElfLoadInfo::parse_elf_data(elf_file_content, argv, envp)?;
    elf_load_info.map_segment_vmos(root_vmar, elf_file_content)?;
    elf_load_info.init_stack(root_vmar, elf_file_content)?;
    debug!("load elf succeeds.");

    Ok(elf_load_info)
}
