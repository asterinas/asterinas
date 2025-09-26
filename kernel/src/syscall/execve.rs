// SPDX-License-Identifier: MPL-2.0

use ostd::arch::cpu::context::UserContext;

use super::{constants::*, SyscallReturn};
use crate::{
    fs::{
        file_table::{get_file_fast, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        path::Path,
    },
    prelude::*,
    process::{check_executable_file, do_execve},
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
) -> Result<Path> {
    let path = if flags.contains(OpenFlags::AT_EMPTY_PATH) && filename.is_empty() {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, dfd);
        file.as_inode_or_err()?.path().clone()
    } else {
        let fs_ref = ctx.thread_local.borrow_fs();
        let fs_resolver = fs_ref.resolver().read();
        let fs_path = FsPath::new(dfd, &filename)?;
        if flags.contains(OpenFlags::AT_SYMLINK_NOFOLLOW) {
            fs_resolver.lookup_no_follow(&fs_path)?
        } else {
            fs_resolver.lookup(&fs_path)?
        }
    };

    check_executable_file(&path)?;

    Ok(path)
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
