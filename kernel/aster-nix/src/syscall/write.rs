// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*, util::read_bytes_from_user};

const STDOUT: u64 = 1;
const STDERR: u64 = 2;

pub fn sys_write(fd: FileDesc, user_buf_ptr: Vaddr, user_buf_len: usize) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, user_buf_len = 0x{:x}",
        fd, user_buf_ptr, user_buf_len
    );

    let file = {
        let current = current!();
        let file_table = current.file_table().lock();
        file_table.get_file(fd)?.clone()
    };

    let mut buffer = vec![0u8; user_buf_len];
    read_bytes_from_user(user_buf_ptr, &mut VmWriter::from(buffer.as_mut_slice()))?;
    debug!("write content = {:?}", buffer);
    let write_len = file.write(&buffer)?;
    Ok(SyscallReturn::Return(write_len as _))
}
