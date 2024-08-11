// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*};

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

    // According to <https://man7.org/linux/man-pages/man2/write.2.html>, if
    // the user specified an empty buffer, we should detect errors by checking
    // the file discriptor. If no errors detected, return 0 successfully.
    let write_len = if user_buf_len != 0 {
        let mut buffer = vec![0u8; user_buf_len];
        CurrentUserSpace::get()
            .read_bytes(user_buf_ptr, &mut VmWriter::from(buffer.as_mut_slice()))?;
        debug!("write content = {:?}", buffer);
        file.write(&buffer)?
    } else {
        file.write(&[])?
    };

    Ok(SyscallReturn::Return(write_len as _))
}
