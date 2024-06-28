// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*, util::write_bytes_to_user};

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

    let mut read_buf = vec![0u8; buf_len];
    let read_len = file.read(&mut read_buf)?;
    write_bytes_to_user(user_buf_addr, &mut VmReader::from(read_buf.as_slice()))?;
    Ok(SyscallReturn::Return(read_len as _))
}
