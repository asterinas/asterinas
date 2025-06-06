// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::prelude::*;

#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[expect(non_camel_case_types)]
pub enum FadviseBehavior {
    POSIX_FADV_NORMAL = 0,
    POSIX_FADV_RANDOM = 1,
    POSIX_FADV_SEQUENTIAL = 2,
    POSIX_FADV_WILLNEED = 3,
    POSIX_FADV_DONTNEED = 4,
    POSIX_FADV_NOREUSE = 5,
}

pub fn sys_fadvise64(
    fd: i32,
    offset: usize,
    len: usize,
    advice: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let behavior = FadviseBehavior::try_from(advice)
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid fadvise behavior:"))?;

    debug!(
        "fd={}, offset={}, len={}, behavior={:?}",
        fd, offset, len, behavior
    );

    let file_table = ctx.thread_local.borrow_file_table();
    let file_table_locked = file_table.unwrap().write();
    let file = file_table_locked.get_file(fd)?;

    match behavior {
        FadviseBehavior::POSIX_FADV_SEQUENTIAL | FadviseBehavior::POSIX_FADV_WILLNEED => {
            let aligned_offset = offset.align_down(PAGE_SIZE);
            let aligned_end = (offset + len).align_up(PAGE_SIZE);
            let aligned_len = aligned_end - aligned_offset;

            if aligned_len > 0 {
                let mut buffer = vec![0u8; aligned_len];

                file.read_bytes_at(aligned_offset, &mut buffer)
                    .map_err(|_| Error::with_message(Errno::EIO, "read failed in fadvise"))?;

                debug!("Preloaded {} bytes for fd: {}", aligned_len, fd);
            }
        }
        _ => todo!(),
    }

    Ok(SyscallReturn::Return(0))
}
