// SPDX-License-Identifier: MPL-2.0

// Modified from protocol.rs in virtio-drivers project
//
// MIT License
//
// Copyright (c) 2022-2023 Ant Group
// Copyright (c) 2019-2020 rCore Developers
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
//
use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use ostd::Pod;

use super::error::{self, SocketError};

pub const VIRTIO_VSOCK_HDR_LEN: usize = core::mem::size_of::<VirtioVsockHdr>();

/// Socket address.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct VsockDeviceAddr {
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
        VirtioVsockOp::try_from(self.op).map_err(|err| err.into())
    }

    pub fn source(&self) -> VsockDeviceAddr {
        VsockDeviceAddr {
            cid: self.src_cid,
            port: self.src_port,
        }
    }

    pub fn destination(&self) -> VsockDeviceAddr {
        VsockDeviceAddr {
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

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
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
