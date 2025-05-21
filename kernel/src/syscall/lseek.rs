// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FileDesc},
        utils::SeekFrom,
    },
    prelude::*,
};

pub fn sys_lseek(fd: FileDesc, offset: isize, whence: u32, ctx: &Context) -> Result<SyscallReturn> {
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
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let offset = file.seek(seek_from)?;
    Ok(SyscallReturn::Return(offset as _))
}
