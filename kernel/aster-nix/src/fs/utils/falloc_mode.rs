// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// Represents the various operation modes for fallocate.
///
/// Each mode determines whether the target disk space within a file
/// will be allocated, deallocated, or zeroed, among other operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallocMode {
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
