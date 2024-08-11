// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*};

pub fn sys_close(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    let file = {
        let mut file_table = ctx.process.file_table().lock();
        let _ = file_table.get_file(fd)?;
        file_table.close_file(fd).unwrap()
    };

    // Cleanup work needs to be done in the `Drop` impl.
    //
    // We don't mind the races between closing the file descriptor and using the file descriptor,
    // because such races are explicitly allowed in the man pages. See the "Multithreaded processes
    // and close()" section in <https://man7.org/linux/man-pages/man2/close.2.html>.
    drop(file);

    // Linux has error codes for the close() system call for diagnostic and remedial purposes, but
    // only for a small subset of file systems such as NFS. We currently have no support for such
    // file systems, so it's fine to just return zero.
    //
    // For details, see the discussion at <https://github.com/asterinas/asterinas/issues/506> and
    // the "Dealing with error returns from close()" section at
    // <https://man7.org/linux/man-pages/man2/close.2.html>.
    Ok(SyscallReturn::Return(0))
}
