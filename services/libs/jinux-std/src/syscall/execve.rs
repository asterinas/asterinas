use jinux_frame::cpu::UserContext;

use super::{constants::*, SyscallReturn};
use crate::fs::file_table::FileDescripter;
use crate::fs::fs_resolver::{FsPath, AT_FDCWD};
use crate::fs::utils::{Dentry, InodeType};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::posix_thread::name::ThreadName;
use crate::process::posix_thread::posix_thread_ext::PosixThreadExt;
use crate::process::program_loader::load_program_to_root_vmar;
use crate::syscall::{SYS_EXECVE, SYS_EXECVEAT};
use crate::util::{read_cstring_from_user, read_val_from_user};

pub fn sys_execve(
    filename_ptr: Vaddr,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    context: &mut UserContext,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EXECVE);
    let executable_path = read_filename(filename_ptr)?;
    let elf_file = {
        let current = current!();
        let fs_resolver = current.fs().read();
        let fs_path = FsPath::new(AT_FDCWD, &executable_path)?;
        fs_resolver.lookup(&fs_path)?
    };
    do_execve(elf_file, argv_ptr_ptr, envp_ptr_ptr, context)?;
    Ok(SyscallReturn::NoReturn)
}

pub fn sys_execveat(
    dfd: FileDescripter,
    filename_ptr: Vaddr,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    flags: u32,
    context: &mut UserContext,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EXECVEAT);
    let flags = OpenFlags::from_bits_truncate(flags);
    let filename = read_filename(filename_ptr)?;
    let elf_file = lookup_executable_file(dfd, filename, flags)?;
    check_file_type_and_permission(&elf_file)?;
    do_execve(elf_file, argv_ptr_ptr, envp_ptr_ptr, context)?;
    Ok(SyscallReturn::NoReturn)
}

fn lookup_executable_file(
    dfd: FileDescripter,
    filename: String,
    flags: OpenFlags,
) -> Result<Arc<Dentry>> {
    let current = current!();
    let fs_resolver = current.fs().read();
    let dentry = if flags.contains(OpenFlags::AT_EMPTY_PATH) && filename.len() == 0 {
        fs_resolver.lookup_from_fd(dfd)
    } else {
        let fs_path = FsPath::new(dfd, &filename)?;
        if flags.contains(OpenFlags::AT_SYMLINK_NOFOLLOW) {
            let dentry = fs_resolver.lookup_no_follow(&fs_path)?;
            if dentry.inode_type() == InodeType::SymLink {
                return_errno_with_message!(Errno::ELOOP, "the executable file is a symlink");
            }
            Ok(dentry)
        } else {
            fs_resolver.lookup(&fs_path)
        }
    }?;
    check_file_type_and_permission(&dentry)?;
    Ok(dentry)
}

fn check_file_type_and_permission(dentry: &Arc<Dentry>) -> Result<()> {
    if !dentry.inode_type().is_reguler_file() {
        return_errno_with_message!(Errno::EACCES, "the dentry is not a regular file");
    }

    if !dentry.inode_mode().is_executable() {
        return_errno_with_message!(Errno::EACCES, "the dentry is not executable");
    }

    Ok(())
}

fn do_execve(
    elf_file: Arc<Dentry>,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    context: &mut UserContext,
) -> Result<()> {
    let executable_path = elf_file.abs_path();
    let argv = read_cstring_vec(argv_ptr_ptr, MAX_ARGV_NUMBER, MAX_ARG_LEN)?;
    let envp = read_cstring_vec(envp_ptr_ptr, MAX_ENVP_NUMBER, MAX_ENV_LEN)?;
    debug!(
        "filename: {:?}, argv = {:?}, envp = {:?}",
        executable_path, argv, envp
    );
    // FIXME: should we set thread name in execve?
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    *posix_thread.thread_name().lock() =
        Some(ThreadName::new_from_executable_path(&executable_path)?);
    // clear ctid
    // FIXME: should we clear ctid when execve?
    *posix_thread.clear_child_tid().lock() = 0;

    let current = current!();
    // destroy root vmars
    let root_vmar = current.root_vmar();
    root_vmar.clear()?;
    current.user_vm().set_default()?;
    // load elf content to new vm space
    let fs_resolver = &*current.fs().read();
    debug!("load program to root vmar");
    let (new_executable_path, elf_load_info) =
        load_program_to_root_vmar(root_vmar, elf_file, argv, envp, fs_resolver, 1)?;
    debug!("load elf in execve succeeds");
    // set executable path
    *current.executable_path().write() = new_executable_path;
    // set signal disposition to default
    current.sig_dispositions().lock().inherit();
    // set cpu context to default
    let default_content = UserContext::default();
    *context.general_regs_mut() = *default_content.general_regs();
    context.set_fsbase(default_content.fsbase());
    *context.fp_regs_mut() = *default_content.fp_regs();
    // set new entry point
    context.set_rip(elf_load_info.entry_point() as _);
    debug!("entry_point: 0x{:x}", elf_load_info.entry_point());
    // set new user stack top
    context.set_rsp(elf_load_info.user_stack_top() as _);
    debug!("user stack top: 0x{:x}", elf_load_info.user_stack_top());
    Ok(())
}

bitflags::bitflags! {
    struct OpenFlags: u32 {
        const AT_EMPTY_PATH = 0x1000;
        const AT_SYMLINK_NOFOLLOW = 0x100;
    }
}

fn read_filename(filename_ptr: Vaddr) -> Result<String> {
    let filename = read_cstring_from_user(filename_ptr, MAX_FILENAME_LEN)?;
    Ok(filename.into_string().unwrap())
}

fn read_cstring_vec(
    array_ptr: Vaddr,
    max_string_number: usize,
    max_string_len: usize,
) -> Result<Vec<CString>> {
    let mut res = Vec::new();
    let mut read_addr = array_ptr;
    let mut find_null = false;
    for _ in 0..max_string_number {
        let cstring_ptr = read_val_from_user::<usize>(read_addr)?;
        read_addr += 8;
        // read a null pointer
        if cstring_ptr == 0 {
            find_null = true;
            break;
        }
        let cstring = read_cstring_from_user(cstring_ptr, max_string_len)?;
        res.push(cstring);
    }
    if !find_null {
        return_errno_with_message!(Errno::E2BIG, "Cannot find null pointer in vector");
    }
    Ok(res)
}
