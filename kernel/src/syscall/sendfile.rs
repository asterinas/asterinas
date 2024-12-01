// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*};

pub fn sys_sendfile(
    out_fd: FileDesc,
    in_fd: FileDesc,
    offset_ptr: Vaddr,
    count: isize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    trace!("raw offset ptr = 0x{:x}", offset_ptr);

    let offset = if offset_ptr == 0 {
        None
    } else {
        let offset: isize = ctx.user_space().read_val(offset_ptr)?;
        if offset < 0 {
            return_errno_with_message!(Errno::EINVAL, "offset cannot be negative");
        }
        Some(offset)
    };

    debug!(
        "out_fd = {}, in_fd = {}, offset = {:x?}, count = 0x{:x}",
        out_fd, in_fd, offset, count
    );

    let mut count = if count < 0 {
        return_errno_with_message!(Errno::EINVAL, "count cannot be negative");
    } else {
        count as usize
    };

    let (out_file, in_file) = {
        let file_table = ctx.posix_thread.file_table().lock();
        let out_file = file_table.get_file(out_fd)?.clone();
        // FIXME: the in_file must support mmap-like operations (i.e., it cannot be a socket).
        let in_file = file_table.get_file(in_fd)?.clone();
        (out_file, in_file)
    };

    // sendfile can send at most `MAX_COUNT` bytes
    const MAX_COUNT: usize = 0x7fff_f000;
    if count > MAX_COUNT {
        count = MAX_COUNT;
    }

    const BUFFER_SIZE: usize = PAGE_SIZE;
    let mut buffer = vec![0u8; BUFFER_SIZE].into_boxed_slice();
    let mut total_len = 0;
    let mut offset = offset.map(|offset| offset as usize);

    while total_len < count {
        // The offset decides how to read from `in_file`.
        // If offset is `Some(_)`, the data will be read from the given offset,
        // and after reading, the file offset of `in_file` will remain unchanged.
        // If offset is `None`, the data will be read from the file offset,
        // and the file offset of `in_file` is adjusted
        // to reflect the number of bytes read from `in_file`.
        let max_readlen = buffer.len().min(count - total_len);

        // Read from `in_file`
        let read_res = if let Some(offset) = offset.as_mut() {
            let res = in_file.read_bytes_at(*offset, &mut buffer[..max_readlen]);
            if let Ok(len) = res.as_ref() {
                *offset += *len;
            }
            res
        } else {
            in_file.read_bytes(&mut buffer[..max_readlen])
        };

        let read_len = match read_res {
            Ok(len) => len,
            Err(e) => {
                if total_len > 0 {
                    warn!("error occurs when trying to read file: {:?}", e);
                    break;
                }
                return Err(e);
            }
        };

        if read_len == 0 {
            break;
        }

        // Note: `sendfile` allows sending partial data,
        // so short reads and short writes are all acceptable
        let write_res = out_file.write_bytes(&buffer[..read_len]);

        match write_res {
            Ok(len) => {
                total_len += len;
                if len < BUFFER_SIZE {
                    break;
                }
            }
            Err(e) => {
                if total_len > 0 {
                    warn!("error occurs when trying to write file: {:?}", e);
                    break;
                }
                return Err(e);
            }
        }
    }

    if let Some(offset) = offset {
        ctx.user_space().write_val(offset_ptr, &(offset as isize))?;
    }

    Ok(SyscallReturn::Return(total_len as _))
}
