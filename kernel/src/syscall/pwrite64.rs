// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    prelude::*,
};

pub fn sys_pwrite64(
    fd: FileDesc,
    user_buf_ptr: Vaddr,
    user_buf_len: usize,
    offset: i64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, user_buf_len = 0x{:x}, offset = 0x{:x}",
        fd, user_buf_ptr, user_buf_len, offset
    );

    if offset < 0 {
        return_errno_with_message!(Errno::EINVAL, "offset cannot be negative");
    }

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    // TODO: Check (f.file->f_mode & FMODE_PWRITE); We don't have f_mode in our FileLike trait
    if user_buf_len == 0 {
        return Ok(SyscallReturn::Return(0));
    }
    if offset.checked_add(user_buf_len as i64).is_none() {
        return_errno_with_message!(Errno::EINVAL, "offset + user_buf_len overflow");
    }

    let user_space = ctx.user_space();
    let mut reader = user_space.reader(user_buf_ptr, user_buf_len)?;
    let write_len = file.write_at(offset as _, &mut reader)?;
    Ok(SyscallReturn::Return(write_len as _))
}
