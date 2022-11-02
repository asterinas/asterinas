use crate::prelude::*;

use crate::syscall::SYS_EXIT;

pub fn sys_exit(exit_code: i32) -> Result<isize> {
    debug!("[syscall][id={}][SYS_EXIT]", SYS_EXIT);
    current!().exit(exit_code);
    Ok(0)
}
