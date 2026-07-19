// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

use super::{SyscallReturn, constants::MAX_FILENAME_LEN};
use crate::{
    fs::file::{
        FsConfigFile,
        file_table::{FileDesc, RawFileDesc, get_file_fast},
    },
    prelude::*,
};

pub fn sys_fsconfig(
    fs_fd: RawFileDesc,
    cmd: u32,
    key_addr: Vaddr,
    value_addr: Vaddr,
    aux: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let fs_fd = FileDesc::try_from(fs_fd)
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid fsconfig fd"))?;
    let cmd = FsConfigOps::try_from(cmd)
        .map_err(|_| Error::with_message(Errno::EOPNOTSUPP, "unsupported fsconfig command"))?;

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fs_fd);
    let fs_config = file
        .downcast_ref::<FsConfigFile>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not a fs context"))?;

    match cmd {
        FsConfigOps::SetFlag => {
            ensure_valid_args(key_addr != 0 && value_addr == 0 && aux == 0)?;
            let key = ctx.user_space().read_cstring(key_addr, MAX_FILENAME_LEN)?;
            fs_config.set_flag(key.to_str()?)?;
        }
        FsConfigOps::SetString => {
            ensure_valid_args(key_addr != 0 && value_addr != 0 && aux == 0)?;
            let key = ctx.user_space().read_cstring(key_addr, MAX_FILENAME_LEN)?;
            let value = ctx
                .user_space()
                .read_cstring(value_addr, MAX_FILENAME_LEN)?;
            fs_config.set_string(key.to_str()?, value.to_str()?)?;
        }
        FsConfigOps::SetBinary => {
            ensure_valid_args(key_addr != 0 && value_addr != 0 && aux > 0)?;
            return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported fsconfig command");
        }
        FsConfigOps::SetPath | FsConfigOps::SetPathEmpty => {
            ensure_valid_args(key_addr != 0 && value_addr != 0 && aux >= 0)?;
            return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported fsconfig command");
        }
        FsConfigOps::SetFd => {
            ensure_valid_args(key_addr != 0 && value_addr == 0 && aux >= 0)?;
            return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported fsconfig command");
        }
        FsConfigOps::Create => {
            ensure_valid_args(key_addr == 0 && value_addr == 0 && aux == 0)?;
            super::fsopen::check_mount_api_capability(ctx)?;
            fs_config.create_fs(ctx)?;
        }
        FsConfigOps::CreateExcl => {
            ensure_valid_args(key_addr == 0 && value_addr == 0 && aux == 0)?;
            return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported fsconfig command");
        }
        FsConfigOps::Reconfigure => {
            ensure_valid_args(key_addr == 0 && value_addr == 0 && aux == 0)?;
            super::fsopen::check_mount_api_capability(ctx)?;
            fs_config.reconfigure_fs(ctx)?;
        }
    }

    Ok(SyscallReturn::Return(0))
}

fn ensure_valid_args(valid: bool) -> Result<()> {
    if !valid {
        return_errno_with_message!(Errno::EINVAL, "invalid fsconfig arguments");
    }
    Ok(())
}

/// The configuration operations supported by `fsconfig`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
enum FsConfigOps {
    /// Sets a boolean parameter.
    SetFlag = 0,
    /// Sets a string parameter.
    SetString = 1,
    /// Sets a binary blob parameter.
    SetBinary = 2,
    /// Sets a path parameter.
    SetPath = 3,
    /// Sets a path parameter that can be empty.
    SetPathEmpty = 4,
    /// Sets a parameter using a file descriptor.
    SetFd = 5,
    /// Creates the filesystem superblock.
    Create = 6,
    /// Reconfigures an existing filesystem superblock.
    Reconfigure = 7,
    /// Creates a new filesystem superblock, failing if reusing an existing one.
    CreateExcl = 8,
}
