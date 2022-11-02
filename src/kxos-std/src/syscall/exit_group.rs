use crate::prelude::*;

use crate::syscall::SYS_EXIT_GROUP;

pub fn sys_exit_group(exit_code: u64) -> Result<isize> {
    debug!("[syscall][id={}][SYS_EXIT_GROUP]", SYS_EXIT_GROUP);
    current!().exit(exit_code as _);
    Ok(0)
}
