use crate::fs::utils::Stat;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;
use crate::util::write_val_to_user;

use super::SyscallReturn;
use super::SYS_LSTAT;

pub fn sys_lstat(filename_ptr: Vaddr, stat_buf_ptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_LSTAT);
    let filename = read_cstring_from_user(filename_ptr, MAX_FILENAME_LEN)?;
    debug!(
        "filename = {:?}, stat_buf_ptr = 0x{:x}",
        filename, stat_buf_ptr
    );
    // TODO: only return a fake result here
    if filename == CString::new(".")? || filename == CString::new("/")? {
        let stat = Stat::fake_dir_stat();
        write_val_to_user(stat_buf_ptr, &stat)?;
        return Ok(SyscallReturn::Return(0));
    }
    todo!()
}
