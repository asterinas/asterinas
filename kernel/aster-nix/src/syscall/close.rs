// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*};

pub fn sys_close(fd: FileDesc) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);
    let current = current!();
    let mut file_table = current.file_table().lock();
    let _ = file_table.get_file(fd)?;
    let file = file_table.close_file(fd).unwrap();
    file.clean_for_close()?;
    Ok(SyscallReturn::Return(0))
}
