// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file::{
        SeekFrom,
        file_table::{RawFileDesc, get_file_fast},
    },
    prelude::*,
};

pub fn sys_lseek(
    raw_fd: RawFileDesc,
    offset: isize,
    whence: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "raw_fd = {}, offset = {}, whence = {}",
        raw_fd, offset, whence
    );

    let seek_from = match SeekType::try_from(whence)? {
        SeekType::SEEK_SET => SeekFrom::Start(offset.cast_unsigned()),
        SeekType::SEEK_CUR => SeekFrom::Current(offset),
        SeekType::SEEK_END => SeekFrom::End(offset),
    };

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);

    let offset = file.seek(seek_from)?;
    Ok(SyscallReturn::Return(offset as _))
}

// Reference: <https://elixir.bootlin.com/linux/v6.17.7/source/include/uapi/linux/fs.h#L52>
#[expect(non_camel_case_types)]
#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
enum SeekType {
    SEEK_SET = 0,
    SEEK_CUR = 1,
    SEEK_END = 2,
}
