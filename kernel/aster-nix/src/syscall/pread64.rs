// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*};

pub fn sys_pread64(
    fd: FileDesc,
    user_buf_ptr: Vaddr,
    user_buf_len: usize,
    offset: i64,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, buf = 0x{:x}, user_buf_len = 0x{:x}, offset = 0x{:x}",
        fd, user_buf_ptr, user_buf_len, offset
    );

    if offset < 0 {
        return_errno_with_message!(Errno::EINVAL, "offset cannot be negative");
    }
    let file = {
        let current = current!();
        let filetable = current.file_table().lock();
        filetable.get_file(fd)?.clone()
    };
    // TODO: Check (f.file->f_mode & FMODE_PREAD); We don't have f_mode in our FileLike trait
    if user_buf_len == 0 {
        return Ok(SyscallReturn::Return(0));
    }
    if offset.checked_add(user_buf_len as i64).is_none() {
        return_errno_with_message!(Errno::EINVAL, "offset + user_buf_len overflow");
    }

    let read_len = {
        let mut buffer = vec![0u8; user_buf_len];
        let read_len = file.read_at(offset as usize, &mut buffer)?;
        CurrentUserSpace::get()
            .write_bytes(user_buf_ptr, &mut VmReader::from(buffer.as_slice()))?;
        read_len
    };

    Ok(SyscallReturn::Return(read_len as _))
}
