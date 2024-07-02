// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;

use crate::prelude::*;

bitflags! {
    /// Operation mode flags for fallocate.
    ///
    /// These flags determine the operation to be performed on the given byte range.
    pub struct FallocateFlags: u32 {
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

/// Represents the various operation modes for fallocate.
///
/// Each mode determines whether the target disk space within a file
/// will be allocated, deallocated, or zeroed, among other operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallocateMode {
    /// Allocates disk space within the range specified.
    Allocate,
    /// Like `Allocate`, but does not change the file size.
    AllocateKeepSize,
    /// Makes shared file data extents private to guarantee subsequent writes.
    AllocateUnshareRange,
    /// Deallocates space (creates a hole) while keeping the file size unchanged.
    PunchHoleKeepSize,
    /// Converts a file range to zeros, expanding the file if necessary.
    ZeroRange,
    /// Like `ZeroRange`, but does not change the file size.
    ZeroRangeKeepSize,
    /// Removes a range of bytes without leaving a hole.
    CollapseRange,
    /// Inserts space within a file without overwriting existing data.
    InsertRange,
}

impl FallocateFlags {
    pub fn from_u32(raw_flags: u32) -> Result<Self> {
        let flags = Self::from_bits(raw_flags)
            .ok_or_else(|| Error::with_message(Errno::EOPNOTSUPP, "invalid flags"))?;

        if flags.contains(Self::FALLOC_FL_PUNCH_HOLE) && flags.contains(Self::FALLOC_FL_ZERO_RANGE)
        {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "PUNCH_HOLE and ZERO_RANGE cannot be used together"
            );
        }
        if flags.contains(Self::FALLOC_FL_PUNCH_HOLE) && !flags.contains(Self::FALLOC_FL_KEEP_SIZE)
        {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "PUNCH_HOLE must be combined with KEEP_SIZE"
            );
        }
        if flags.contains(Self::FALLOC_FL_COLLAPSE_RANGE)
            && !(flags - Self::FALLOC_FL_COLLAPSE_RANGE).is_empty()
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "COLLAPSE_RANGE must be used exclusively without any other flags"
            );
        }
        if flags.contains(Self::FALLOC_FL_INSERT_RANGE)
            && !(flags - Self::FALLOC_FL_INSERT_RANGE).is_empty()
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "INSERT_RANGE must be used exclusively without any other flags"
            );
        }
        if flags.contains(Self::FALLOC_FL_UNSHARE_RANGE)
            && !(flags - (Self::FALLOC_FL_UNSHARE_RANGE | Self::FALLOC_FL_KEEP_SIZE)).is_empty()
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "UNSHARE_RANGE can only be combined with KEEP_SIZE."
            );
        }

        Ok(flags)
    }
}

impl From<FallocateFlags> for FallocateMode {
    fn from(flags: FallocateFlags) -> FallocateMode {
        match (
            flags.contains(FallocateFlags::FALLOC_FL_PUNCH_HOLE),
            flags.contains(FallocateFlags::FALLOC_FL_ZERO_RANGE),
            flags.contains(FallocateFlags::FALLOC_FL_COLLAPSE_RANGE),
            flags.contains(FallocateFlags::FALLOC_FL_INSERT_RANGE),
            flags.contains(FallocateFlags::FALLOC_FL_UNSHARE_RANGE),
            flags.contains(FallocateFlags::FALLOC_FL_KEEP_SIZE),
        ) {
            (true, _, _, _, _, _) => FallocateMode::PunchHoleKeepSize,
            (_, true, _, _, _, true) => FallocateMode::ZeroRangeKeepSize,
            (_, true, _, _, _, false) => FallocateMode::ZeroRange,
            (_, _, true, _, _, _) => FallocateMode::CollapseRange,
            (_, _, _, true, _, _) => FallocateMode::InsertRange,
            (_, _, _, _, true, _) => FallocateMode::AllocateUnshareRange,
            (_, _, _, _, _, true) => FallocateMode::AllocateKeepSize,
            _ => FallocateMode::Allocate,
        }
    }
}
