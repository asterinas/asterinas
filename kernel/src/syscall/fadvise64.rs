// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::get_file_fast, prelude::*};

#[repr(i32)]
#[derive(Debug, TryFromInt)]
enum FadviseBehavior {
    Normal = 0,
    Random = 1,
    Sequential = 2,
    Willneed = 3,
    Dontneed = 4,
    Noreuse = 5,
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

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let _file = get_file_fast!(&mut file_table, fd);

    match behavior {
        FadviseBehavior::Normal => {
            warn!("POSIX_FADV_NORMAL is ignored");
        }
        FadviseBehavior::Random => {
            warn!("POSIX_FADV_RANDOM is ignored");
        }
        FadviseBehavior::Sequential => {
            warn!("POSIX_FADV_SEQUENTIAL is ignored");
        }
        FadviseBehavior::Willneed => {
            warn!("POSIX_FADV_WILLNEED is ignored");
        }
        FadviseBehavior::Dontneed => {
            warn!("POSIX_FADV_DONTNEED is ignored");
        }
        FadviseBehavior::Noreuse => {
            warn!("POSIX_FADV_NOREUSE is ignored");
        }
    }

    Ok(SyscallReturn::Return(0))
}
