// SPDX-License-Identifier: MPL-2.0

use super::super::{SyscallReturn, constants::MAX_FILENAME_LEN};
use crate::{
    fs::file::{
        FsConfigFile,
        file_table::{RawFileDesc, get_file_fast},
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
    if fs_fd < 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid fsconfig fd");
    }
    let cmd = FsConfigOps::try_from(cmd)?;

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fs_fd.try_into()?);
    let fs_config = file
        .downcast_ref::<FsConfigFile>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not a fs context"))?;

    match cmd {
        FsConfigOps::SetFlag => {
            if key_addr == 0 {
                return_errno_with_message!(Errno::EINVAL, "fsconfig key is NULL");
            }
            if value_addr != 0 || aux != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid fsconfig arguments");
            }
            let key = ctx.user_space().read_cstring(key_addr, MAX_FILENAME_LEN)?;
            fs_config.set_flag(key.to_str()?)?;
        }
        FsConfigOps::SetString => {
            if key_addr == 0 {
                return_errno_with_message!(Errno::EINVAL, "fsconfig key is NULL");
            }
            if value_addr == 0 || aux != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid fsconfig arguments");
            }
            let key = ctx.user_space().read_cstring(key_addr, MAX_FILENAME_LEN)?;
            let value = ctx
                .user_space()
                .read_cstring(value_addr, MAX_FILENAME_LEN)?;
            fs_config.set_string(key.to_str()?, value.to_str()?)?;
        }
        FsConfigOps::SetBinary => {
            if key_addr == 0 || value_addr == 0 || aux <= 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid fsconfig arguments");
            }
            return_errno_with_message!(Errno::EOPNOTSUPP, "fsconfig command not supported");
        }
        FsConfigOps::SetPath | FsConfigOps::SetPathEmpty => {
            if key_addr == 0 || value_addr == 0 || aux < 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid fsconfig arguments");
            }
            return_errno_with_message!(Errno::EOPNOTSUPP, "fsconfig command not supported");
        }
        FsConfigOps::SetFd => {
            if key_addr == 0 || value_addr != 0 || aux < 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid fsconfig arguments");
            }
            return_errno_with_message!(Errno::EOPNOTSUPP, "fsconfig command not supported");
        }
        FsConfigOps::Create => {
            if key_addr != 0 || value_addr != 0 || aux != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid fsconfig arguments");
            }
            fs_config.create_fs(ctx)?;
        }
        FsConfigOps::Reconfigure => {
            if key_addr != 0 || value_addr != 0 || aux != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid fsconfig arguments");
            }
            fs_config.reconfigure_fs(ctx)?;
        }
    }

    Ok(SyscallReturn::Return(0))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FsConfigOps {
    SetFlag,
    SetString,
    SetBinary,
    SetPath,
    SetPathEmpty,
    SetFd,
    Create,
    Reconfigure,
}

impl TryFrom<u32> for FsConfigOps {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        Ok(match value {
            0 => Self::SetFlag,
            1 => Self::SetString,
            2 => Self::SetBinary,
            3 => Self::SetPath,
            4 => Self::SetPathEmpty,
            5 => Self::SetFd,
            6 => Self::Create,
            7 => Self::Reconfigure,
            _ => return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported fsconfig command"),
        })
    }
}
