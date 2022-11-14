use super::{constants::*, SyscallReturn};
use crate::{memory::read_cstring_from_user, prelude::*, syscall::SYS_ACCESS};

pub fn sys_access(filename_ptr: Vaddr, file_mode: u64) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_ACCESS]", SYS_ACCESS);
    let filename = read_cstring_from_user(filename_ptr, MAX_FILENAME_LEN)?;
    debug!("filename: {:?}, file_mode = {}", filename, file_mode);
    // TODO: access currenly does not check and just return success
    Ok(SyscallReturn::Return(0))
}
