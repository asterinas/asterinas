// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file_table::FileDesc, utils::SeekFrom},
    prelude::*,
};

pub fn sys_lseek(
    fd: FileDesc,
    offset: isize,
    whence: u32,
    _ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("fd = {}, offset = {}, whence = {}", fd, offset, whence);
    let seek_from = match whence {
        0 => {
            if offset < 0 {
                return_errno!(Errno::EINVAL);
            }
            SeekFrom::Start(offset as usize)
        }
        1 => SeekFrom::Current(offset),
        2 => SeekFrom::End(offset),
        _ => return_errno!(Errno::EINVAL),
    };
    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    let offset = file.seek(seek_from)?;
    Ok(SyscallReturn::Return(offset as _))
}
