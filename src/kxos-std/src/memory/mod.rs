pub mod aux_vec;
pub mod elf;
pub mod init_stack;
pub mod mmap_area;
pub mod user_heap;
pub mod vm_page;
use alloc::ffi::CString;
use kxos_frame::{
    debug,
    vm::{Pod, Vaddr, VmIo, VmSpace},
};

use crate::process::Process;

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

/// copy bytes from user space of current process. The bytes len is the len of dest.
pub fn read_bytes_from_user(src: Vaddr, dest: &mut [u8]) {
    let current = Process::current();
    let vm_space = current
        .vm_space()
        .expect("[Internal error]Current should have vm space to copy bytes from user");
    vm_space.read_bytes(src, dest).expect("read bytes failed");
}

/// copy val (Plain of Data type) from user space of current process.
pub fn read_val_from_user<T: Pod>(src: Vaddr) -> T {
    let current = Process::current();
    let vm_space = current
        .vm_space()
        .expect("[Internal error]Current should have vm space to copy val from user");
    vm_space.read_val(src).expect("read val failed")
}

/// write bytes from user space of current process. The bytes len is the len of src.
pub fn write_bytes_to_user(dest: Vaddr, src: &[u8]) {
    let current = Process::current();
    let vm_space = current
        .vm_space()
        .expect("[Internal error]Current should have vm space to write bytes to user");
    vm_space.write_bytes(dest, src).expect("write bytes failed")
}

/// write val (Plain of Data type) to user space of current process.
pub fn write_val_to_user<T: Pod>(dest: Vaddr, val: &T) {
    let current = Process::current();
    let vm_space = current
        .vm_space()
        .expect("[Internal error]Current should have vm space to write val to user");
    vm_space.write_val(dest, val).expect("write val failed");
}
