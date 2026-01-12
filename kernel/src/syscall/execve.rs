// SPDX-License-Identifier: MPL-2.0

use ostd::arch::cpu::context::UserContext;

use super::{SyscallReturn, constants::*};
use crate::{
    fs::{
        file_table::FileDesc,
        path::{AT_FDCWD, FsPath, Path},
    },
    prelude::*,
    process::do_execve,
};

pub fn sys_execve(
    filename_ptr: Vaddr,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    ctx: &Context,
    user_context: &mut UserContext,
) -> Result<SyscallReturn> {
    let elf_file = {
        let flags = OpenFlags::empty();
        lookup_executable_file(AT_FDCWD, filename_ptr, flags, ctx)?
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
        let flags = OpenFlags::from_bits(flags)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
        lookup_executable_file(dfd, filename_ptr, flags, ctx)?
    };

    do_execve(elf_file, argv_ptr_ptr, envp_ptr_ptr, ctx, user_context)?;
    Ok(SyscallReturn::NoReturn)
}

fn lookup_executable_file(
    dfd: FileDesc,
    filename_ptr: Vaddr,
    flags: OpenFlags,
    ctx: &Context,
) -> Result<Path> {
    let filename = ctx
        .user_space()
        .read_cstring(filename_ptr, MAX_FILENAME_LEN)?;

    let path = {
        let filename = filename.to_string_lossy();
        let fs_path = if flags.contains(OpenFlags::AT_EMPTY_PATH) && filename.is_empty() {
            FsPath::from_fd(dfd)?
        } else {
            FsPath::from_fd_and_path(dfd, &filename)?
        };

        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        if flags.contains(OpenFlags::AT_SYMLINK_NOFOLLOW) {
            path_resolver.lookup_no_follow(&fs_path)?
        } else {
            path_resolver.lookup(&fs_path)?
        }
    };

    Ok(path)
}

bitflags::bitflags! {
    struct OpenFlags: u32 {
        const AT_EMPTY_PATH = 0x1000;
        const AT_SYMLINK_NOFOLLOW = 0x100;
    }
}
