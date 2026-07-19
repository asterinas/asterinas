// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs,
    fs::file::file_table::{RawFileDesc, get_file_fast},
    prelude::*,
};

pub fn sys_pread64(
    raw_fd: RawFileDesc,
    user_buf_ptr: Vaddr,
    user_buf_len: usize,
    offset: i64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "raw_fd = {}, buf = 0x{:x}, user_buf_len = 0x{:x}, offset = 0x{:x}",
        raw_fd, user_buf_ptr, user_buf_len, offset
    );

    if offset < 0 {
        return_errno_with_message!(Errno::EINVAL, "offset cannot be negative");
    }

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);

    if offset.checked_add(user_buf_len as i64).is_none() {
        return_errno_with_message!(Errno::EINVAL, "offset + user_buf_len overflow");
    }

    let read_len = {
        let user_space = ctx.user_space();
        let mut writer = user_space.writer(user_buf_ptr, user_buf_len)?;
        file.read_at(offset as usize, &mut writer)?
    };

    if read_len > 0 {
        fs::vfs::notify::on_access(&file);
    }

    Ok(SyscallReturn::Return(read_len as _))
}
