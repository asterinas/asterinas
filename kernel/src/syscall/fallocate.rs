// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file_table::FileDesc, utils::FallocMode},
    prelude::*,
    process::ResourceType,
};

pub fn sys_fallocate(
    fd: FileDesc,
    mode: u64,
    offset: i64,
    len: i64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, mode = {}, offset = {}, len = {}",
        fd, mode, offset, len
    );

    check_offset_and_len(offset, len, ctx)?;

    let file = {
        let file_table = ctx.process.file_table().lock();
        file_table.get_file(fd)?.clone()
    };

    let falloc_mode = FallocMode::try_from(
        RawFallocMode::from_bits(mode as _)
            .ok_or_else(|| Error::with_message(Errno::EOPNOTSUPP, "invalid fallocate mode"))?,
    )?;
    file.fallocate(falloc_mode, offset as usize, len as usize)?;

    Ok(SyscallReturn::Return(0))
}

fn check_offset_and_len(offset: i64, len: i64, ctx: &Context) -> Result<()> {
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
        let resource_limits = ctx.process.resource_limits().lock();
        resource_limits
            .get_rlimit(ResourceType::RLIMIT_FSIZE)
            .get_cur() as usize
    };
    if (offset + len) as usize > max_file_size {
        return_errno_with_message!(Errno::EFBIG, "offset+len exceeds the maximum file size");
    }
    Ok(())
}

bitflags! {
    /// Operation mode flags for fallocate.
    ///
    /// These flags determine the operation to be performed on the given byte range.
    struct RawFallocMode: u32 {
        /// File size will not be changed when extending the file.
        const FALLOC_FL_KEEP_SIZE = 0x01;
        /// De-allocates a range (creates a hole).
        ///
        /// Must be OR-ed with `FALLOC_FL_KEEP_SIZE`.
        const FALLOC_FL_PUNCH_HOLE = 0x02;
        /// Removes a range of a file without leaving a hole.
        ///
        /// The offset and length must be multiples of the filesystem block size.
        const FALLOC_FL_COLLAPSE_RANGE = 0x08;
        /// Converts a range of a file to zeros.
        ///
        /// Preallocates blocks within the range, converting to unwritten extents.
        const FALLOC_FL_ZERO_RANGE = 0x10;
        /// Inserts space within the file size without overwriting any existing data.
        ///
        /// The offset and length must be multiples of the filesystem block size.
        const FALLOC_FL_INSERT_RANGE = 0x20;
        /// Unshares shared blocks within the file size without overwriting any existing data.
        ///
        /// Guarantees that subsequent writes will not fail due to lack of space.
        const FALLOC_FL_UNSHARE_RANGE = 0x40;
    }
}

impl TryFrom<RawFallocMode> for FallocMode {
    type Error = crate::error::Error;
    fn try_from(raw_mode: RawFallocMode) -> Result<Self> {
        // Check for invalid combinations of flags
        if raw_mode.contains(RawFallocMode::FALLOC_FL_PUNCH_HOLE)
            && raw_mode.contains(RawFallocMode::FALLOC_FL_ZERO_RANGE)
        {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "PUNCH_HOLE and ZERO_RANGE cannot be used together"
            );
        }
        if raw_mode.contains(RawFallocMode::FALLOC_FL_PUNCH_HOLE)
            && !raw_mode.contains(RawFallocMode::FALLOC_FL_KEEP_SIZE)
        {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "PUNCH_HOLE must be combined with KEEP_SIZE"
            );
        }
        if raw_mode.contains(RawFallocMode::FALLOC_FL_COLLAPSE_RANGE)
            && !(raw_mode - RawFallocMode::FALLOC_FL_COLLAPSE_RANGE).is_empty()
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "COLLAPSE_RANGE must be used exclusively without any other flags"
            );
        }
        if raw_mode.contains(RawFallocMode::FALLOC_FL_INSERT_RANGE)
            && !(raw_mode - RawFallocMode::FALLOC_FL_INSERT_RANGE).is_empty()
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "INSERT_RANGE must be used exclusively without any other flags"
            );
        }
        if raw_mode.contains(RawFallocMode::FALLOC_FL_UNSHARE_RANGE)
            && !(raw_mode
                - (RawFallocMode::FALLOC_FL_UNSHARE_RANGE | RawFallocMode::FALLOC_FL_KEEP_SIZE))
                .is_empty()
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "UNSHARE_RANGE can only be combined with KEEP_SIZE."
            );
        }

        // Transform valid flags into the fallocate mode
        let mode = if raw_mode.contains(RawFallocMode::FALLOC_FL_PUNCH_HOLE) {
            FallocMode::PunchHoleKeepSize
        } else if raw_mode.contains(RawFallocMode::FALLOC_FL_ZERO_RANGE) {
            if raw_mode.contains(RawFallocMode::FALLOC_FL_KEEP_SIZE) {
                FallocMode::ZeroRangeKeepSize
            } else {
                FallocMode::ZeroRange
            }
        } else if raw_mode.contains(RawFallocMode::FALLOC_FL_COLLAPSE_RANGE) {
            FallocMode::CollapseRange
        } else if raw_mode.contains(RawFallocMode::FALLOC_FL_INSERT_RANGE) {
            FallocMode::InsertRange
        } else if raw_mode.contains(RawFallocMode::FALLOC_FL_UNSHARE_RANGE) {
            FallocMode::AllocateUnshareRange
        } else if raw_mode.contains(RawFallocMode::FALLOC_FL_KEEP_SIZE) {
            FallocMode::AllocateKeepSize
        } else {
            FallocMode::Allocate
        };
        Ok(mode)
    }
}
