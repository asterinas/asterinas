// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file_table::FileDesc, utils::SeekFrom},
    prelude::*,
    util::write_bytes_to_user,
};

pub fn sys_pread64(fd: FileDesc, buf_ptr: Vaddr, count: usize, pos: i64) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, buf = 0x{:x}, count = 0x{:x}, pos = 0x{:x}",
        fd, buf_ptr, count, pos
    );

    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;

    let seek_from = SeekFrom::Start(pos as usize);
    file.seek(seek_from)?;

    let read_len = {
        let mut buffer = vec![0u8; count];
        let read_len = file.read(&mut buffer)?;
        write_bytes_to_user(buf_ptr, &buffer)?;
        read_len
    };

    Ok(SyscallReturn::Return(read_len as _))
}
