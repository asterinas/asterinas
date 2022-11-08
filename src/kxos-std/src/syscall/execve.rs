use kxos_frame::cpu::CpuContext;

use super::{constants::*, SyscallReturn};
use crate::process::elf::load_elf_to_vm_space;
use crate::{memory::read_bytes_from_user, prelude::*, process::Process, syscall::SYS_EXECVE};

pub fn sys_execve(
    filename_ptr: Vaddr,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    context: &mut CpuContext,
) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_EXECVE]", SYS_EXECVE);
    let mut filename_buffer = vec![0u8; MAX_FILENAME_LEN];
    read_bytes_from_user(filename_ptr, &mut filename_buffer)?;
    let filename = CString::from(CStr::from_bytes_until_nul(&filename_buffer).unwrap());
    debug!("filename: {:?}", filename);

    if filename != CString::new("./hello").unwrap() {
        panic!("Unknown filename.");
    }
    let elf_file_content = crate::user_apps::read_execve_hello_content();
    let current = Process::current();
    // Set process vm space to default
    let vm_space = current
        .vm_space()
        .expect("[Internal Error] User process should have vm space");
    vm_space.clear();
    let user_vm = current
        .user_vm()
        .expect("[Internal Error] User process should have user vm");
    user_vm.set_default();
    // load elf content to new vm space
    let elf_load_info =
        load_elf_to_vm_space(filename, elf_file_content, &vm_space).expect("load elf failed");
    debug!("load elf in execve succeeds");
    // set signal disposition to default
    current.sig_dispositions().lock().inherit();
    // set cpu context to default
    let defalut_content = CpuContext::default();
    context.gp_regs = defalut_content.gp_regs;
    context.fs_base = defalut_content.fs_base;
    context.fp_regs = defalut_content.fp_regs;
    // set new entry point
    context.gp_regs.rip = elf_load_info.entry_point();
    debug!("entry_point: 0x{:x}", elf_load_info.entry_point());
    // set new user stack top
    context.gp_regs.rsp = elf_load_info.user_stack_top();
    debug!("user stack top: 0x{:x}", elf_load_info.user_stack_top());
    Ok(SyscallReturn::NoReturn)
}
