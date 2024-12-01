// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*};

pub fn sys_write(
    fd: FileDesc,
    user_buf_ptr: Vaddr,
    user_buf_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, user_buf_len = 0x{:x}",
        fd, user_buf_ptr, user_buf_len
    );

    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(fd)?.clone()
    };

    // According to <https://man7.org/linux/man-pages/man2/write.2.html>, if
    // the user specified an empty buffer, we should detect errors by checking
    // the file descriptor. If no errors detected, return 0 successfully.
    let write_len = if user_buf_len != 0 {
        let mut reader = ctx
            .process
            .root_vmar()
            .vm_space()
            .reader(user_buf_ptr, user_buf_len)?;
        file.write(&mut reader)?
    } else {
        file.write_bytes(&[])?
    };

    Ok(SyscallReturn::Return(write_len as _))
}
