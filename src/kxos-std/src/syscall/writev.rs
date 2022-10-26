use crate::prelude::*;

use crate::{
    memory::{read_bytes_from_user, read_val_from_user},
    syscall::SYS_WRITEV,
};

use super::SyscallResult;

const IOVEC_MAX: usize = 256;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct IoVec {
    base: Vaddr,
    len: usize,
}

pub fn sys_writev(fd: u64, io_vec_addr: u64, io_vec_count: u64) -> SyscallResult {
    debug!("[syscall][id={}][SYS_WRITEV]", SYS_WRITEV);
    let res = do_sys_writev(fd, io_vec_addr as Vaddr, io_vec_count as usize);
    SyscallResult::Return(res as _)
}

pub fn do_sys_writev(fd: u64, io_vec_addr: Vaddr, io_vec_count: usize) -> usize {
    debug!("fd = {}", fd);
    debug!("io_vec_addr = 0x{:x}", io_vec_addr);
    debug!("io_vec_counter = 0x{:x}", io_vec_count);
    let mut write_len = 0;
    for i in 0..io_vec_count {
        let io_vec = read_val_from_user::<IoVec>(io_vec_addr + i * 8);
        let base = io_vec.base;
        let len = io_vec.len;
        debug!("base = 0x{:x}", base);
        debug!("len = {}", len);
        let mut buffer = vec![0u8; len];
        read_bytes_from_user(base, &mut buffer);
        let content = alloc::str::from_utf8(&buffer).unwrap();
        write_len += len;
        if fd == 1 {
            info!("User Mode Message: {}", content);
        } else if fd == 2 {
            info!("User Mode Error Message: {}", content);
        } else {
            info!("content = {}", content);
            panic!("Unsupported fd {}", fd);
        }
    }
    write_len
}
