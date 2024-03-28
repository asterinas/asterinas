// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;
use pod::Pod;

use super::error::{self, SocketError};

pub const VIRTIO_VSOCK_HDR_LEN: usize = core::mem::size_of::<VirtioVsockHdr>();

/// Socket address.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct VsockAddr {
    /// Context Identifier.
    pub cid: u64,
    /// Port number.
    pub port: u32,
}

/// VirtioVsock header precedes the payload in each packet.
// #[repr(packed)]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioVsockHdr {
    pub src_cid: u64,
    pub dst_cid: u64,
    pub src_port: u32,
    pub dst_port: u32,

    pub len: u32,
    pub socket_type: u16,
    pub op: u16,
    pub flags: u32,
    /// Total receive buffer space for this socket. This includes both free and in-use buffers.
    pub buf_alloc: u32,
    /// Free-running bytes received counter.
    pub fwd_cnt: u32,
}

impl Default for VirtioVsockHdr {
    fn default() -> Self {
        Self {
            src_cid: 0,
            dst_cid: 0,
            src_port: 0,
            dst_port: 0,
            len: 0,
            socket_type: VsockType::Stream as u16,
            op: 0,
            flags: 0,
            buf_alloc: 0,
            fwd_cnt: 0,
        }
    }
}

impl VirtioVsockHdr {
    /// Returns the length of the data.
    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn op(&self) -> error::Result<VirtioVsockOp> {
        self.op.try_into()
    }

    pub fn source(&self) -> VsockAddr {
        VsockAddr {
            cid: self.src_cid,
            port: self.src_port,
        }
    }

    pub fn destination(&self) -> VsockAddr {
        VsockAddr {
            cid: self.dst_cid,
            port: self.dst_port,
        }
    }

    pub fn check_data_is_empty(&self) -> error::Result<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(SocketError::UnexpectedDataInPacket)
        }
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum VirtioVsockOp {
    #[default]
    Invalid = 0,

    /* Connect operations */
    Request = 1,
    Response = 2,
    Rst = 3,
    Shutdown = 4,

    /* To send payload */
    Rw = 5,

    /* Tell the peer our credit info */
    CreditUpdate = 6,
    /* Request the peer to send the credit info to us */
    CreditRequest = 7,
}

/// TODO: This could be optimized by upgrading [int_to_c_enum::TryFromIntError] to carrying the invalid int number
impl TryFrom<u16> for VirtioVsockOp {
    type Error = SocketError;

    fn try_from(v: u16) -> Result<Self, Self::Error> {
        let op = match v {
            0 => Self::Invalid,
            1 => Self::Request,
            2 => Self::Response,
            3 => Self::Rst,
            4 => Self::Shutdown,
            5 => Self::Rw,
            6 => Self::CreditUpdate,
            7 => Self::CreditRequest,
            _ => return Err(SocketError::UnknownOperation(v)),
        };
        Ok(op)
    }
}

bitflags! {
    #[repr(C)]
    #[derive(Default, Pod)]
    /// Header flags field type makes sense when connected socket receives VIRTIO_VSOCK_OP_SHUTDOWN.
    pub struct ShutdownFlags: u32{
        /// The peer will not receive any more data.
        const VIRTIO_VSOCK_SHUTDOWN_RCV = 1 << 0;
        /// The peer will not send any more data.
        const VIRTIO_VSOCK_SHUTDOWN_SEND = 1 << 1;
        /// The peer will not send or receive any more data.
        const VIRTIO_VSOCK_SHUTDOWN_ALL = Self::VIRTIO_VSOCK_SHUTDOWN_RCV.bits | Self::VIRTIO_VSOCK_SHUTDOWN_SEND.bits;
    }
}

/// Currently only stream sockets are supported. type is 1 for stream socket types.
#[derive(Copy, Clone, Debug)]
#[repr(u16)]
pub enum VsockType {
    /// Stream sockets provide in-order, guaranteed, connection-oriented delivery without message boundaries.
    Stream = 1,
    /// seqpacket socket type introduced in virtio-v1.2.
    SeqPacket = 2,
}
