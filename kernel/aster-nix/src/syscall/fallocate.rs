// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file_table::FileDesc, utils::FallocateFlags},
    prelude::*,
    process::ResourceType,
};

pub fn sys_fallocate(fd: FileDesc, mode: u64, offset: i64, len: i64) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, mode = {}, offset = {}, len = {}",
        fd, mode, offset, len
    );

    check_offset_and_len(offset, len)?;

    let file = {
        let current = current!();
        let file_table = current.file_table().lock();
        file_table.get_file(fd)?.clone()
    };

    let flags = FallocateFlags::from_u32(mode as _)?;
    file.fallocate(flags, offset as usize, len as usize)?;

    Ok(SyscallReturn::Return(0))
}

#[inline]
fn check_offset_and_len(offset: i64, len: i64) -> Result<()> {
    if offset < 0 || len <= 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "offset is less than 0, or len is less than or equal to 0"
        );
    }
    if offset.checked_add(len).is_none() {
        return_errno_with_message!(Errno::EINVAL, "offset+len has overflowed");
    }

    let max_file_size = {
        let current = current!();
        let resource_limits = current.resource_limits().lock();
        resource_limits
            .get_rlimit(ResourceType::RLIMIT_FSIZE)
            .get_cur() as usize
    };
    if (offset + len) as usize > max_file_size {
        return_errno_with_message!(Errno::EFBIG, "offset+len exceeds the maximum file size");
    }
    Ok(())
}
