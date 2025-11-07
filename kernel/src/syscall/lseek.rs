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

    let seek_from = match SeekType::try_from(whence)? {
        SeekType::SEEK_SET => SeekFrom::Start(offset.cast_unsigned()),
        SeekType::SEEK_CUR => SeekFrom::Current(offset),
        SeekType::SEEK_END => SeekFrom::End(offset),
    };

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let offset = file.seek(seek_from)?;
    Ok(SyscallReturn::Return(offset as _))
}

// Reference: <https://elixir.bootlin.com/linux/v6.17.7/source/include/uapi/linux/fs.h#L52>
#[derive(Clone, Copy, Debug, TryFromInt)]
#[repr(u32)]
#[expect(non_camel_case_types)]
enum SeekType {
    SEEK_SET = 0,
    SEEK_CUR = 1,
    SEEK_END = 2,
}
