// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file_table::FileDesc, utils::IoctlCmd},
    prelude::*,
};

pub fn sys_ioctl(fd: FileDesc, cmd: u32, arg: Vaddr) -> Result<SyscallReturn> {
    let ioctl_cmd = IoctlCmd::try_from(cmd)?;
    debug!(
        "fd = {}, ioctl_cmd = {:?}, arg = 0x{:x}",
        fd, ioctl_cmd, arg
    );
    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    let res = file.ioctl(ioctl_cmd, arg)?;
    Ok(SyscallReturn::Return(res as _))
}
