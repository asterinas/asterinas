use crate::fs::file_table::FileDescripter;
use crate::{log_syscall_entry, prelude::*};

use crate::syscall::SYS_WRITEV;
use crate::util::{read_bytes_from_user, read_val_from_user};

use super::SyscallReturn;

const IOVEC_MAX: usize = 256;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct IoVec {
    base: Vaddr,
    len: usize,
}

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
    for i in 0..io_vec_count {
        let io_vec = read_val_from_user::<IoVec>(io_vec_ptr + i * core::mem::size_of::<IoVec>())?;
        if io_vec.base == 0 {
            continue;
        }
        let buffer = {
            let base = io_vec.base;
            let len = io_vec.len;
            let mut buffer = vec![0u8; len];
            read_bytes_from_user(base, &mut buffer)?;
            buffer
        };
        let write_len = file.write(&buffer)?;
        total_len += write_len;
    }
    Ok(total_len)
}
