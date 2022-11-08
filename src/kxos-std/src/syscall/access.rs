use super::{constants::*, SyscallReturn};
use crate::{memory::read_bytes_from_user, prelude::*, syscall::SYS_ACCESS};

pub fn sys_access(filename_ptr: Vaddr, file_mode: u64) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_ACCESS]", SYS_ACCESS);
    let mut filename_buffer = vec![0u8; MAX_FILENAME_LEN];
    read_bytes_from_user(filename_ptr, &mut filename_buffer);
    let filename = CString::from(CStr::from_bytes_until_nul(&filename_buffer).unwrap());
    debug!("filename: {:?}", filename);
    warn!("access currenly does not check and just return success");
    Ok(SyscallReturn::Return(0))
}
