use crate::memory::write_bytes_to_user;
use crate::{fs::file::FileDescripter, prelude::*};

use super::SyscallReturn;
use super::SYS_READ;

pub fn sys_read(fd: FileDescripter, user_buf_addr: Vaddr, buf_len: usize) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_READ]", SYS_READ);
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, buf_len = 0x{:x}",
        fd, user_buf_addr, buf_len
    );
    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd);
    match file {
        None => return_errno!(Errno::EBADF),
        Some(file) => {
            let mut read_buf = vec![0u8; buf_len];
            let read_len = file.read(&mut read_buf)?;
            write_bytes_to_user(user_buf_addr, &read_buf)?;
            debug!(
                "read_len = {}, read_buf = {:?}",
                read_len,
                &read_buf[..read_len]
            );
            // let read_str = core::str::from_utf8(&read_buf[..read_len - 1]).unwrap();
            // println!("str = {}" ,read_str);
            // todo!();
            return Ok(SyscallReturn::Return(read_len as _));
        }
    }
}
