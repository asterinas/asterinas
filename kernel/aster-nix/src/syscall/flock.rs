// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        inode_handle::InodeHandle,
        utils::{Flock, FlockOps, FlockType},
    },
    prelude::*,
};

pub fn sys_flock(fd: FileDesc, ops: i32) -> Result<SyscallReturn> {
    debug!("flock: fd: {}, ops: {:?}", fd, ops);

    let file = {
        let current = current!();
        let file_table = current.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let inode_file = file
        .downcast_ref::<InodeHandle>()
        .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
    let ops: FlockOps = FlockOps::from_i32(ops)?;
    if ops.contains(FlockOps::LOCK_UN) {
        inode_file.unlock_flock();
    } else {
        let is_nonblocking = ops.contains(FlockOps::LOCK_NB);
        let flock = {
            let type_ = FlockType::from(ops);
            Flock::new(&file, type_)
        };
        inode_file.set_flock(flock, is_nonblocking)?;
    }
    Ok(SyscallReturn::Return(0))
}
