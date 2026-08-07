// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;

bitflags! {
    /// Per-I/O flags for `preadv2` and `pwritev2`, mirroring the `RWF_*`
    /// flags in Linux. Like the corresponding `O_*` open flags, they apply
    /// to a single I/O rather than the whole open file description.
    pub struct RwfFlags: u32 {
        /// High-priority request. Linux may poll for completion to reduce
        /// latency.
        const RWF_HIPRI = 0x00000001;
        /// Per-I/O `O_DSYNC`: commit data before the I/O returns.
        const RWF_DSYNC = 0x00000002;
        /// Per-I/O `O_SYNC`: commit data and metadata before the I/O returns.
        const RWF_SYNC = 0x00000004;
        /// Do not block: return `EAGAIN` if the operation would wait, e.g.
        /// on page I/O, allocation, or lock contention.
        const RWF_NOWAIT = 0x00000008;
    }
}
