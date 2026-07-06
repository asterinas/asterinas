// SPDX-License-Identifier: MPL-2.0

mod read;
mod write;

pub(super) use read::IoUringReadRequest;
pub(super) use write::IoUringWriteRequest;

// If offs is set to -1, the offset will use (and advance) the file position.
// Reference: IORING_OP_WRITE in https://man7.org/linux/man-pages/man2/io_uring_enter.2.html
const CURRENT_FILE_OFFSET: u64 = u64::MAX;
