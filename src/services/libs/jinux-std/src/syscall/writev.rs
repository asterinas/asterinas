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

pub fn sys_writev(fd: u64, io_vec_ptr: u64, io_vec_count: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_WRITEV);
    let res = do_sys_writev(fd, io_vec_ptr as Vaddr, io_vec_count as usize)?;
    Ok(SyscallReturn::Return(res as _))
}

pub fn do_sys_writev(fd: u64, io_vec_ptr: Vaddr, io_vec_count: usize) -> Result<usize> {
    debug!(
        "fd = {}, io_vec_ptr = 0x{:x}, io_vec_counter = 0x{:x}",
        fd, io_vec_ptr, io_vec_count
    );
    let mut write_len = 0;
    for i in 0..io_vec_count {
        let io_vec = read_val_from_user::<IoVec>(io_vec_ptr + i * 8)?;
        let base = io_vec.base;
        let len = io_vec.len;
        let mut buffer = vec![0u8; len];
        read_bytes_from_user(base, &mut buffer)?;
        let content = alloc::str::from_utf8(&buffer).unwrap();
        write_len += len;
        if fd == 1 {
            print!("{}", content);
        } else if fd == 2 {
            print!("{}", content);
        } else {
            info!("content = {}", content);
            panic!("Unsupported fd {}", fd);
        }
    }
    Ok(write_len)
}
