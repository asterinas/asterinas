// SPDX-License-Identifier: MPL-2.0

//! Wire-format virtio-vsock headers and protocol enums.

use bitflags::bitflags;
use int_to_c_enum::TryFromInt;

/// The socket type encoded in a virtio-vsock packet.
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum VirtioVsockType {
    /// Identifies a byte-stream vsock connection.
    Stream = 1,
}

/// The operation encoded in a virtio-vsock packet.
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum VirtioVsockOp {
    /// Starts a new connection.
    Request = 1,
    /// Accepts a connection request.
    Response = 2,
    /// Resets a connection.
    Rst = 3,
    /// Announces half-close state changes.
    Shutdown = 4,
    /// Carries stream payload bytes.
    Rw = 5,
    /// Updates the peer-visible receive credit.
    CreditUpdate = 6,
    /// Requests an immediate credit update from the peer.
    CreditRequest = 7,
}

bitflags! {
    /// The half-close bits carried by `VirtioVsockOp::Shutdown` packets.
    pub struct VirtioVsockShutdownFlags: u32 {
        /// Indicates that the sender will no longer receive more data.
        const RECEIVE = 1;
        /// Indicates that the sender will no longer send more data.
        const SEND = 2;
    }
}

/// The common header of a virtio-vsock packet.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct VirtioVsockHdr {
    /// The source CID.
    pub src_cid: u64,
    /// The destination CID.
    pub dst_cid: u64,
    /// The source port.
    pub src_port: u32,
    /// The destination port.
    pub dst_port: u32,
    /// The payload length in bytes.
    pub len: u32,
    /// The encoded [`VirtioVsockType`].
    pub type_: u16,
    /// The encoded [`VirtioVsockOp`].
    pub op: u16,
    /// Stores operation-specific flags.
    pub flags: u32,
    /// The sender's advertised receive buffer size.
    pub buf_alloc: u32,
    /// The sender's forwarded-byte counter.
    pub fwd_cnt: u32,
}

impl VirtioVsockHdr {
    /// Creates a stream-type virtio-vsock header.
    #[expect(
        clippy::too_many_arguments,
        reason = "the wire header fields map directly to the virtio-vsock specification"
    )]
    pub const fn new(
        src_cid: u64,
        dst_cid: u64,
        src_port: u32,
        dst_port: u32,
        len: u32,
        op: VirtioVsockOp,
        flags: u32,
        buf_alloc: u32,
        fwd_cnt: u32,
    ) -> Self {
        Self {
            src_cid,
            dst_cid,
            src_port,
            dst_port,
            len,
            type_: VirtioVsockType::Stream as u16,
            op: op as u16,
            flags,
            buf_alloc,
            fwd_cnt,
        }
    }

    /// Decodes and returns the packet operation.
    pub fn op(&self) -> Option<VirtioVsockOp> {
        VirtioVsockOp::try_from(self.op).ok()
    }
}

/// The event identifier carried by the transport event queue.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(super) enum VirtioVsockEventId {
    /// Indicates that the transport has been reset.
    TransportReset = 0,
}

/// The payload stored in the transport event virtqueue.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct VirtioVsockEvent {
    /// The encoded [`VirtioVsockEventId`].
    pub id: u32,
}
