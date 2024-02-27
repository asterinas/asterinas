// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;

bitflags! {
    pub struct StatusFlags: u32 {
        /// append on each write
        const O_APPEND = 1 << 10;
        /// non block
        const O_NONBLOCK = 1 << 11;
        /// synchronized I/O, data
        const O_DSYNC = 1 << 12;
        /// signal-driven I/O
        const O_ASYNC = 1 << 13;
        /// direct I/O
        const O_DIRECT = 1 << 14;
        /// on x86_64, O_LARGEFILE is 0
        /// not update st_atime
        const O_NOATIME = 1 << 18;
        /// synchronized I/O, data and metadata
        const O_SYNC = 1 << 20;
        /// equivalent of POSIX.1's O_EXEC
        const O_PATH = 1 << 21;
    }
}
