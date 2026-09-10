// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;

bitflags! {
    /// Per-I/O flags for `preadv2`/`pwritev2`, mirroring Linux `RWF_*`;
    /// `RWF_HIPRI`, `RWF_DSYNC`, and `RWF_SYNC` are silently ignored.
    pub struct RwfFlags: u32 {
        /// high-priority request
        const RWF_HIPRI = 0x00000001;
        /// synchronized I/O, data (per-I/O `O_DSYNC`)
        const RWF_DSYNC = 0x00000002;
        /// synchronized I/O, data and metadata (per-I/O `O_SYNC`)
        const RWF_SYNC = 0x00000004;
        /// do not wait; return `EAGAIN` if the operation would block
        const RWF_NOWAIT = 0x00000008;
    }
}
