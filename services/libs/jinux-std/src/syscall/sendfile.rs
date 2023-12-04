use crate::fs::file_table::FileDescripter;
use crate::fs::utils::SeekFrom;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::{read_val_from_user, write_val_to_user};

use super::{SyscallReturn, SYS_SENDFILE};

pub fn sys_sendfile(
    out_fd: FileDescripter,
    in_fd: FileDescripter,
    offset_ptr: Vaddr,
    count: usize,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SENDFILE);

    trace!("raw offset ptr = 0x{:x}", offset_ptr);

    let offset = if offset_ptr == 0 {
        None
    } else {
        Some(read_val_from_user::<isize>(offset_ptr)?)
    };

    debug!(
        "out_fd = {}, in_fd = {}, offset = {:x?}, count = 0x{:x}",
        out_fd, in_fd, offset, count
    );

    let (out_file, in_file) = {
        let current = current!();
        let file_table = current.file_table().lock();
        let out_file = file_table.get_file(out_fd)?.clone();
        let in_file = file_table.get_file(in_fd)?.clone();
        (out_file, in_file)
    };

    if let Some(offset) = offset {
        in_file.seek(SeekFrom::Start(offset as usize))?;
    }

    if count == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let mut buffer = vec![0u8; count];
    let read_len = in_file.read(&mut buffer)?;

    if read_len == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let write_len = out_file.write(&buffer[..read_len])?;
    Ok(SyscallReturn::Return(write_len as _))
}
