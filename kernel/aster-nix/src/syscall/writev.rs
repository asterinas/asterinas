// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDescripter, log_syscall_entry, prelude::*, syscall::SYS_WRITEV,
    util::IoVecIter,
};

pub fn sys_writev(
    fd: FileDescripter,
    io_vec_ptr: Vaddr,
    io_vec_count: usize,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_WRITEV);
    let res = do_sys_writev(fd, io_vec_ptr, io_vec_count)?;
    Ok(SyscallReturn::Return(res as _))
}

fn do_sys_writev(fd: FileDescripter, io_vec_ptr: Vaddr, io_vec_count: usize) -> Result<usize> {
    debug!(
        "fd = {}, io_vec_ptr = 0x{:x}, io_vec_counter = 0x{:x}",
        fd, io_vec_ptr, io_vec_count
    );
    let file = {
        let current = current!();
        let filetable = current.file_table().lock();
        filetable.get_file(fd)?.clone()
    };

    let mut total_len = 0;

    let io_vec_iter = IoVecIter::new(io_vec_ptr, io_vec_count);

    for io_vec in io_vec_iter {
        let io_vec = io_vec?;

        let buffer = {
            let mut buffer = vec![0u8; io_vec.len()];
            io_vec.read_from_user(&mut buffer)?;
            buffer
        };

        let write_len = file.write(&buffer)?;
        total_len += write_len;
    }
    Ok(total_len)
}
