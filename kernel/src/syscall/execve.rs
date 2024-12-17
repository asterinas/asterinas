// SPDX-License-Identifier: MPL-2.0

use aster_rights::WriteOp;
use ostd::{
    cpu::{FpuState, RawGeneralRegs, UserContext},
    user::UserContextApi,
};

use super::{constants::*, SyscallReturn};
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
        path::Dentry,
        utils::InodeType,
    },
    prelude::*,
    process::{
        check_executable_file, load_program_to_vm, posix_thread::ThreadName, Credentials, Process,
        MAX_ARGV_NUMBER, MAX_ARG_LEN, MAX_ENVP_NUMBER, MAX_ENV_LEN,
    },
};

pub fn sys_execve(
    filename_ptr: Vaddr,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    ctx: &Context,
    user_context: &mut UserContext,
) -> Result<SyscallReturn> {
    let elf_file = {
        let executable_path = read_filename(filename_ptr, ctx)?;
        lookup_executable_file(AT_FDCWD, executable_path, OpenFlags::empty(), ctx)?
    };

    do_execve(elf_file, argv_ptr_ptr, envp_ptr_ptr, ctx, user_context)?;
    Ok(SyscallReturn::NoReturn)
}

pub fn sys_execveat(
    dfd: FileDesc,
    filename_ptr: Vaddr,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
    user_context: &mut UserContext,
) -> Result<SyscallReturn> {
    let elf_file = {
        let flags = OpenFlags::from_bits_truncate(flags);
        let filename = read_filename(filename_ptr, ctx)?;
        lookup_executable_file(dfd, filename, flags, ctx)?
    };

    do_execve(elf_file, argv_ptr_ptr, envp_ptr_ptr, ctx, user_context)?;
    Ok(SyscallReturn::NoReturn)
}

fn lookup_executable_file(
    dfd: FileDesc,
    filename: String,
    flags: OpenFlags,
    ctx: &Context,
) -> Result<Dentry> {
    let fs_resolver = ctx.posix_thread.fs().resolver().read();
    let dentry = if flags.contains(OpenFlags::AT_EMPTY_PATH) && filename.is_empty() {
        fs_resolver.lookup_from_fd(dfd)
    } else {
        let fs_path = FsPath::new(dfd, &filename)?;
        if flags.contains(OpenFlags::AT_SYMLINK_NOFOLLOW) {
            let dentry = fs_resolver.lookup_no_follow(&fs_path)?;
            if dentry.type_() == InodeType::SymLink {
                return_errno_with_message!(Errno::ELOOP, "the executable file is a symlink");
            }
            Ok(dentry)
        } else {
            fs_resolver.lookup(&fs_path)
        }
    }?;
    check_executable_file(&dentry)?;
    Ok(dentry)
}

fn do_execve(
    elf_file: Dentry,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    ctx: &Context,
    user_context: &mut UserContext,
) -> Result<()> {
    let Context {
        process,
        posix_thread,
        thread: _,
        task: _,
    } = ctx;

    let executable_path = elf_file.abs_path();
    let argv = read_cstring_vec(argv_ptr_ptr, MAX_ARGV_NUMBER, MAX_ARG_LEN, ctx)?;
    let envp = read_cstring_vec(envp_ptr_ptr, MAX_ENVP_NUMBER, MAX_ENV_LEN, ctx)?;
    debug!(
        "filename: {:?}, argv = {:?}, envp = {:?}",
        executable_path, argv, envp
    );
    // FIXME: should we set thread name in execve?
    *posix_thread.thread_name().lock() =
        Some(ThreadName::new_from_executable_path(&executable_path)?);
    // clear ctid
    // FIXME: should we clear ctid when execve?
    *posix_thread.clear_child_tid().lock() = 0;

    // Ensure that the file descriptors with the close-on-exec flag are closed.
    // FIXME: This is just wrong if the file table is shared with other processes.
    let closed_files = posix_thread.file_table().lock().close_files_on_exec();
    drop(closed_files);

    debug!("load program to root vmar");
    let (new_executable_path, elf_load_info) = {
        let fs_resolver = &*posix_thread.fs().resolver().read();
        let process_vm = process.vm();
        load_program_to_vm(process_vm, elf_file.clone(), argv, envp, fs_resolver, 1)?
    };

    // After the program has been successfully loaded, the virtual memory of the current process
    // is initialized. Hence, it is necessary to clear the previously recorded robust list.
    *posix_thread.robust_list().lock() = None;
    debug!("load elf in execve succeeds");

    let credentials = ctx.posix_thread.credentials_mut();
    set_uid_from_elf(process, &credentials, &elf_file)?;
    set_gid_from_elf(process, &credentials, &elf_file)?;

    // set executable path
    process.set_executable_path(new_executable_path);
    // set signal disposition to default
    process.sig_dispositions().lock().inherit();
    // set cpu context to default
    *user_context.general_regs_mut() = RawGeneralRegs::default();
    user_context.set_tls_pointer(0);
    *user_context.fpu_state_mut() = FpuState::default();
    // FIXME: how to reset the FPU state correctly? Before returning to the user space,
    // the kernel will call `handle_pending_signal`, which may update the CPU states so that
    // when the kernel switches to the user mode, the control of the CPU will be handed over
    // to the user-registered signal handlers.
    user_context.fpu_state().restore();
    // set new entry point
    user_context.set_instruction_pointer(elf_load_info.entry_point() as _);
    debug!("entry_point: 0x{:x}", elf_load_info.entry_point());
    // set new user stack top
    user_context.set_stack_pointer(elf_load_info.user_stack_top() as _);
    debug!("user stack top: 0x{:x}", elf_load_info.user_stack_top());
    Ok(())
}

bitflags::bitflags! {
    struct OpenFlags: u32 {
        const AT_EMPTY_PATH = 0x1000;
        const AT_SYMLINK_NOFOLLOW = 0x100;
    }
}

fn read_filename(filename_ptr: Vaddr, ctx: &Context) -> Result<String> {
    let filename = ctx
        .user_space()
        .read_cstring(filename_ptr, MAX_FILENAME_LEN)?;
    Ok(filename.into_string().unwrap())
}

fn read_cstring_vec(
    array_ptr: Vaddr,
    max_string_number: usize,
    max_string_len: usize,
    ctx: &Context,
) -> Result<Vec<CString>> {
    let mut res = Vec::new();
    // On Linux, argv pointer and envp pointer can be specified as NULL.
    if array_ptr == 0 {
        return Ok(res);
    }
    let mut read_addr = array_ptr;
    let mut find_null = false;
    let user_space = ctx.user_space();
    for _ in 0..max_string_number {
        let cstring_ptr = user_space.read_val::<usize>(read_addr)?;
        read_addr += 8;
        // read a null pointer
        if cstring_ptr == 0 {
            find_null = true;
            break;
        }
        let cstring = user_space.read_cstring(cstring_ptr, max_string_len)?;
        res.push(cstring);
    }
    if !find_null {
        return_errno_with_message!(Errno::E2BIG, "Cannot find null pointer in vector");
    }
    Ok(res)
}

/// Sets uid for credentials as the same of uid of elf file if elf file has `set_uid` bit.
fn set_uid_from_elf(
    current: &Process,
    credentials: &Credentials<WriteOp>,
    elf_file: &Dentry,
) -> Result<()> {
    if elf_file.mode()?.has_set_uid() {
        let uid = elf_file.owner()?;
        credentials.set_euid(uid);

        current.clear_parent_death_signal();
    }

    // No matter whether the elf_file has `set_uid` bit, suid should be reset.
    credentials.reset_suid();
    Ok(())
}

/// Sets gid for credentials as the same of gid of elf file if elf file has `set_gid` bit.
fn set_gid_from_elf(
    current: &Process,
    credentials: &Credentials<WriteOp>,
    elf_file: &Dentry,
) -> Result<()> {
    if elf_file.mode()?.has_set_gid() {
        let gid = elf_file.group()?;
        credentials.set_egid(gid);

        current.clear_parent_death_signal();
    }

    // No matter whether the the elf file has `set_gid` bit, sgid should be reset.
    credentials.reset_sgid();
    Ok(())
}
