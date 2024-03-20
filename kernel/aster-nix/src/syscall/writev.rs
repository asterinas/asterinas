// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*, util::copy_iovs_from_user};

pub fn sys_writev(fd: FileDesc, io_vec_ptr: Vaddr, io_vec_count: usize) -> Result<SyscallReturn> {
    let res = do_sys_writev(fd, io_vec_ptr, io_vec_count)?;
    Ok(SyscallReturn::Return(res as _))
}

fn do_sys_writev(fd: FileDesc, io_vec_ptr: Vaddr, io_vec_count: usize) -> Result<usize> {
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

    let io_vecs = copy_iovs_from_user(io_vec_ptr, io_vec_count)?;
    for io_vec in io_vecs.as_ref() {
        if io_vec.is_empty() {
            continue;
        }

        let buffer = {
            let mut buffer = vec![0u8; io_vec.len()];
            io_vec.read_exact_from_user(&mut buffer)?;
            buffer
        };

        // FIXME: According to the man page
        // at <https://man7.org/linux/man-pages/man2/readv.2.html>,
        // writev must be atomic,
        // but the current implementation does not ensure atomicity.
        // A suitable fix would be to add a `writev` method for the `FileLike` trait,
        // allowing each subsystem to implement atomicity.
        let write_len = file.write(&buffer)?;
        total_len += write_len;
    }
    Ok(total_len)
}
