// SPDX-License-Identifier: MPL-2.0

use super::{constants::*, SyscallReturn};
use crate::{log_syscall_entry, prelude::*, syscall::SYS_ACCESS, util::read_cstring_from_user};

pub fn sys_access(filename_ptr: Vaddr, file_mode: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_ACCESS);
    let filename = read_cstring_from_user(filename_ptr, MAX_FILENAME_LEN)?;
    debug!("filename: {:?}, file_mode = {}", filename, file_mode);
    // TODO: access currenly does not check and just return success
    Ok(SyscallReturn::Return(0))
}
