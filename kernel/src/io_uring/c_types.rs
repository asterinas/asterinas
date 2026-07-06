// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

bitflags! {
    /// `IORING_SETUP_*` flags in Linux.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L171-L253>.
    pub struct IoUringSetupFlags: u32 {
        const IOPOLL = 1 << 0;
        const SQPOLL = 1 << 1;
        const SQ_AFF = 1 << 2;
        const CQSIZE = 1 << 3;
        const CLAMP = 1 << 4;
        const ATTACH_WQ = 1 << 5;
        const R_DISABLED = 1 << 6;
        const SUBMIT_ALL = 1 << 7;
        const COOP_TASKRUN = 1 << 8;
        const TASKRUN_FLAG = 1 << 9;
        const SQE128 = 1 << 10;
        const CQE32 = 1 << 11;
        const SINGLE_ISSUER = 1 << 12;
        const DEFER_TASKRUN = 1 << 13;
        const NO_MMAP = 1 << 14;
        const REGISTERED_FD_ONLY = 1 << 15;
        const NO_SQARRAY = 1 << 16;
        const HYBRID_IOPOLL = 1 << 17;
        const CQE_MIXED = 1 << 18;
        const SQE_MIXED = 1 << 19;
        const SQ_REWIND = 1 << 20;

        const SUPPORTED = Self::CQSIZE.bits
            | Self::CLAMP.bits
            | Self::SUBMIT_ALL.bits;
    }
}

impl IoUringSetupFlags {
    pub fn from_user_bits(bits: u32) -> Result<Self> {
        let flags = IoUringSetupFlags::from_bits(bits)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown io_uring_setup flags"))?;
        if !flags.difference(IoUringSetupFlags::SUPPORTED).is_empty() {
            return_errno_with_message!(Errno::EINVAL, "unsupported io_uring_setup flags");
        }
        Ok(flags)
    }
}

bitflags! {
    /// `IORING_ENTER_*` flags in Linux.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L599-L609>.
    pub struct IoUringEnterFlags: u32 {
        const GETEVENTS = 1 << 0;
        const SQ_WAKEUP = 1 << 1;
        const SQ_WAIT = 1 << 2;
        const EXT_ARG = 1 << 3;
        const REGISTERED_RING = 1 << 4;
        const ABS_TIMER = 1 << 5;
        const EXT_ARG_REG = 1 << 6;
        const NO_IOWAIT = 1 << 7;

        const UNSUPPORTED = Self::SQ_WAKEUP.bits
            | Self::SQ_WAIT.bits
            | Self::EXT_ARG.bits
            | Self::REGISTERED_RING.bits
            | Self::ABS_TIMER.bits
            | Self::EXT_ARG_REG.bits
            | Self::NO_IOWAIT.bits;
    }
}

impl IoUringEnterFlags {
    pub fn from_user_bits(bits: u32) -> Result<Self> {
        let flags = Self::from_bits(bits)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown io_uring_enter flags"))?;
        if flags.intersects(Self::UNSUPPORTED) {
            return_errno_with_message!(Errno::EINVAL, "unsupported io_uring_enter flags");
        }
        Ok(flags)
    }
}

bitflags! {
    /// `io_uring_params->features` bits in Linux.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L630-L647>.
    pub struct IoUringFeatures: u32 {
        const SINGLE_MMAP = 1 << 0;
        const NODROP = 1 << 1;
        const SUBMIT_STABLE = 1 << 2;
        const RW_CUR_POS = 1 << 3;
        const CUR_PERSONALITY = 1 << 4;
        const FAST_POLL = 1 << 5;
        const POLL_32BITS = 1 << 6;
        const SQPOLL_NONFIXED = 1 << 7;
        const EXT_ARG = 1 << 8;
        const NATIVE_WORKERS = 1 << 9;
        const RSRC_TAGS = 1 << 10;
        const CQE_SKIP = 1 << 11;
        const LINKED_FILE = 1 << 12;
        const REG_REG_RING = 1 << 13;
        const RECVSEND_BUNDLE = 1 << 14;
        const MIN_TIMEOUT = 1 << 15;
        const RW_ATTR = 1 << 16;
        const NO_IOWAIT = 1 << 17;
    }
}

/// The fixed header of `struct io_rings` in Linux, before `cqes[]`.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/linux/io_uring_types.h#L156-L224>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct IoRingMeta {
    pub sq_head: u32,
    pub sq_tail: u32,
    pub cq_head: u32,
    pub cq_tail: u32,
    pub sq_ring_mask: u32,
    pub cq_ring_mask: u32,
    pub sq_ring_entries: u32,
    pub cq_ring_entries: u32,
    pub sq_dropped: u32,
    pub sq_flags: u32,
    pub cq_flags: u32,
    pub cq_overflow: u32,
    pub padding: [u8; 16],
}

impl IoRingMeta {
    pub const fn new(sq_entries: u32, cq_entries: u32) -> Self {
        Self {
            sq_head: 0,
            sq_tail: 0,
            cq_head: 0,
            cq_tail: 0,
            sq_ring_mask: sq_entries - 1,
            cq_ring_mask: cq_entries - 1,
            sq_ring_entries: sq_entries,
            cq_ring_entries: cq_entries,
            sq_dropped: 0,
            sq_flags: 0,
            cq_flags: 0,
            cq_overflow: 0,
            padding: [0; 16],
        }
    }
}

/// `struct io_sqring_offsets` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L561-L571>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct SqRingOffsets {
    pub head: u32,
    pub tail: u32,
    pub ring_mask: u32,
    pub ring_entries: u32,
    pub flags: u32,
    pub dropped: u32,
    pub array: u32,
    pub resv1: u32,
    pub user_addr: u64,
}

/// `struct io_cqring_offsets` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L580-L590>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct CqRingOffsets {
    pub head: u32,
    pub tail: u32,
    pub ring_mask: u32,
    pub ring_entries: u32,
    pub overflow: u32,
    pub cqes: u32,
    pub flags: u32,
    pub resv1: u32,
    pub user_addr: u64,
}

/// `struct io_uring_params` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L614-L625>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct IoUringParams {
    pub sq_entries: u32,
    pub cq_entries: u32,
    pub flags: u32,
    pub sq_thread_cpu: u32,
    pub sq_thread_idle: u32,
    pub features: u32,
    pub wq_fd: u32,
    pub resv: [u32; 3],
    pub sq_off: SqRingOffsets,
    pub cq_off: CqRingOffsets,
}

/// `struct io_uring_sqe` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L32-L120>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct IoUringSqe {
    pub opcode: u8,
    pub flags: u8,
    pub ioprio: u16,
    pub fd: i32,
    pub off: u64,
    pub addr: u64,
    pub len: u32,
    pub rw_flags: u32,
    pub user_data: u64,
    pub buf_index: u16,
    pub personality: u16,
    pub splice_fd_in: i32,
    pub addr3: u64,
    pub pad2: [u64; 1],
}

bitflags! {
    /// `sqe->flags` bits in Linux.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L143-L169>.
    pub struct IoUringSqeFlags: u8 {
        const FIXED_FILE = 1 << 0;
        const IO_DRAIN = 1 << 1;
        const IO_LINK = 1 << 2;
        const IO_HARDLINK = 1 << 3;
        const ASYNC = 1 << 4;
        const BUFFER_SELECT = 1 << 5;
        const CQE_SKIP_SUCCESS = 1 << 6;

        const SUPPORTED = Self::ASYNC.bits;
    }
}

/// `struct io_uring_cqe` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L500-L510>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct IoUringCqe {
    pub user_data: u64,
    pub res: i32,
    pub flags: u32,
}

// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/io_uring/io_uring.h#L170-L171>.
pub const MAX_SQ_ENTRIES: u32 = 32 * 1024;
pub const MAX_CQ_ENTRIES: u32 = 2 * MAX_SQ_ENTRIES;

// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L548-L553>.
pub const IORING_OFF_SQ_RING: usize = 0;
pub const IORING_OFF_CQ_RING: usize = 0x0800_0000;
pub const IORING_OFF_SQES: usize = 0x1000_0000;

/// `enum io_uring_op` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/include/uapi/linux/io_uring.h#L255-L324>.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, TryFromInt)]
pub enum IoUringOpcode {
    Nop = 0,
    Read = 22,
    Write = 23,
}
