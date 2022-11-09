use crate::prelude::*;

use crate::{
    memory::{read_bytes_from_user, write_bytes_to_user},
    syscall::SYS_READLINK,
};

use super::SyscallReturn;

const MAX_FILENAME_LEN: usize = 128;

pub fn sys_readlink(
    filename_ptr: u64,
    user_buf_ptr: u64,
    user_buf_len: u64,
) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_READLINK]", SYS_READLINK);
    let res = do_sys_readlink(
        filename_ptr as Vaddr,
        user_buf_ptr as Vaddr,
        user_buf_len as usize,
    )?;
    Ok(SyscallReturn::Return(res as _))
}

/// do sys readlink
/// write the content to user buffer, returns the actual write len
pub fn do_sys_readlink(
    filename_ptr: Vaddr,
    user_buf_ptr: Vaddr,
    user_buf_len: usize,
) -> Result<usize> {
    debug!(
        "filename ptr = 0x{:x}, user_buf_ptr = 0x{:x}, user_buf_len = 0x{:x}",
        filename_ptr, user_buf_ptr, user_buf_len
    );

    let mut filename_buffer = [0u8; MAX_FILENAME_LEN];
    let current = current!();
    read_bytes_from_user(filename_ptr, &mut filename_buffer)?;
    let filename = CStr::from_bytes_until_nul(&filename_buffer).expect("Invalid filename");
    debug!("filename = {:?}", filename);
    if filename == CString::new("/proc/self/exe").unwrap().as_c_str() {
        // "proc/self/exe" is used to read the filename of current executable
        let process_file_name = current.filename().unwrap();
        debug!("process exec filename= {:?}", process_file_name);
        let bytes = process_file_name.as_bytes_with_nul();
        let bytes_len = bytes.len();
        let write_len = bytes_len.min(user_buf_len);

        write_bytes_to_user(user_buf_ptr, &bytes[..write_len])?;
        return Ok(write_len);
    }

    panic!("does not support linkname other than /proc/self/exe")
}
