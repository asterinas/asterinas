// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*};

pub fn sys_read(
    fd: FileDesc,
    user_buf_addr: Vaddr,
    buf_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, buf_len = 0x{:x}",
        fd, user_buf_addr, buf_len
    );

    let file = {
        let file_table = ctx.process.file_table().lock();
        file_table.get_file(fd)?.clone()
    };

    // According to <https://man7.org/linux/man-pages/man2/read.2.html>, if
    // the user specified an empty buffer, we should detect errors by checking
    // the file descriptor. If no errors detected, return 0 successfully.
    let read_len = if buf_len != 0 {
        let mut writer = ctx
            .process
            .root_vmar()
            .vm_space()
            .writer(user_buf_addr, buf_len)?;
        file.read(&mut writer)?
    } else {
        file.read_bytes(&mut [])?
    };

    Ok(SyscallReturn::Return(read_len as _))
}
