// SPDX-License-Identifier: MPL-2.0

use core::cmp::min;

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*};

pub fn sys_read(fd: FileDesc, user_buf_addr: Vaddr, buf_len: usize) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, buf_len = 0x{:x}",
        fd, user_buf_addr, buf_len
    );

    let file = {
        let current = current!();
        let file_table = current.file_table().lock();
        file_table.get_file(fd)?.clone()
    };

    // According to <https://man7.org/linux/man-pages/man2/read.2.html>, if
    // the user specified an empty buffer, we should detect errors by checking
    // the file discriptor. If no errors detected, return 0 successfully.
    let read_len = if buf_len != 0 {
        let mut read_buf = vec![0u8; buf_len];
        let read_len = file.read(&mut read_buf)?;
        CurrentUserSpace::get().write_bytes(
            user_buf_addr,
            &mut VmReader::from(&read_buf[..min(read_len, buf_len)]),
        )?;
        read_len
    } else {
        file.read(&mut [])?
    };

    Ok(SyscallReturn::Return(read_len as _))
}
