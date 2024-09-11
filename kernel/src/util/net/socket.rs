// SPDX-License-Identifier: MPL-2.0

use super::read_socket_addr_from_user;
use crate::{
    net::socket::SocketAddr,
    prelude::*,
    util::{net::write_socket_addr_with_max_len, VmReaderArray, VmWriterArray},
};

/// Standard well-defined IP protocols.
/// From https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/in.h.
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
pub enum Protocol {
    IPPROTO_IP = 0,         /* Dummy protocol for TCP		*/
    IPPROTO_ICMP = 1,       /* Internet Control Message Protocol	*/
    IPPROTO_IGMP = 2,       /* Internet Group Management Protocol	*/
    IPPROTO_TCP = 6,        /* Transmission Control Protocol	*/
    IPPROTO_EGP = 8,        /* Exterior Gateway Protocol		*/
    IPPROTO_PUP = 12,       /* PUP protocol				*/
    IPPROTO_UDP = 17,       /* User Datagram Protocol		*/
    IPPROTO_IDP = 22,       /* XNS IDP protocol			*/
    IPPROTO_TP = 29,        /* SO Transport Protocol Class 4	*/
    IPPROTO_DCCP = 33,      /* Datagram Congestion Control Protocol */
    IPPROTO_IPV6 = 41,      /* IPv6-in-IPv4 tunnelling		*/
    IPPROTO_RSVP = 46,      /* RSVP Protocol			*/
    IPPROTO_GRE = 47,       /* Cisco GRE tunnels (rfc 1701,1702)	*/
    IPPROTO_ESP = 50,       /* Encapsulation Security Payload protocol */
    IPPROTO_AH = 51,        /* Authentication Header protocol	*/
    IPPROTO_MTP = 92,       /* Multicast Transport Protocol		*/
    IPPROTO_BEETPH = 94,    /* IP option pseudo header for BEET	*/
    IPPROTO_ENCAP = 98,     /* Encapsulation Header			*/
    IPPROTO_PIM = 103,      /* Protocol Independent Multicast	*/
    IPPROTO_COMP = 108,     /* Compression Header Protocol		*/
    IPPROTO_SCTP = 132,     /* Stream Control Transport Protocol	*/
    IPPROTO_UDPLITE = 136,  /* UDP-Lite (RFC 3828)			*/
    IPPROTO_MPLS = 137,     /* MPLS in IP (RFC 4023)		*/
    IPPROTO_ETHERNET = 143, /* Ethernet-within-IPv6 Encapsulation	*/
    IPPROTO_RAW = 255,      /* Raw IP packets			*/
    IPPROTO_MPTCP = 262,    /* Multipath TCP connection		*/
}

/// Socket types.
/// From https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/net.h
#[repr(i32)]
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum SockType {
    /// Stream socket
    SOCK_STREAM = 1,
    /// Datagram socket
    SOCK_DGRAM = 2,
    /// Raw socket
    SOCK_RAW = 3,
    /// Reliably-delivered message
    SOCK_RDM = 4,
    /// Sequential packet socket
    SOCK_SEQPACKET = 5,
    /// Datagram Congestion Control Protocol socket
    SOCK_DCCP = 6,
    /// Linux specific way of getting packets at the dev level
    SOCK_PACKET = 10,
}

pub const SOCK_TYPE_MASK: i32 = 0xf;

bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    pub struct SockFlags: i32 {
        const SOCK_NONBLOCK = 1 << 11;
        const SOCK_CLOEXEC = 1 << 19;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CUserMsgHdr {
    /// Pointer to socket address structure
    pub msg_name: Vaddr,
    /// Size of socket address
    pub msg_namelen: i32,
    /// Scatter/Gather iov array
    pub msg_iov: Vaddr,
    /// The # of elements in msg_iov
    pub msg_iovlen: u32,
    /// Ancillary data
    pub msg_control: Vaddr,
    /// Ancillary data buffer length
    pub msg_controllen: u32,
    /// Flags on received message
    pub msg_flags: u32,
}

impl CUserMsgHdr {
    pub fn read_socket_addr_from_user(&self) -> Result<Option<SocketAddr>> {
        if self.msg_name == 0 {
            return Ok(None);
        }

        let socket_addr = read_socket_addr_from_user(self.msg_name, self.msg_namelen as usize)?;
        Ok(Some(socket_addr))
    }

    pub fn write_socket_addr_to_user(&self, addr: &SocketAddr) -> Result<()> {
        if self.msg_name == 0 {
            return Ok(());
        }

        write_socket_addr_with_max_len(addr, self.msg_name, self.msg_namelen)?;
        Ok(())
    }

    pub fn copy_reader_array_from_user<'a>(&self, ctx: &'a Context) -> Result<VmReaderArray<'a>> {
        VmReaderArray::from_user_io_vecs(ctx, self.msg_iov, self.msg_iovlen as usize)
    }

    pub fn copy_writer_array_from_user<'a>(&self, ctx: &'a Context) -> Result<VmWriterArray<'a>> {
        VmWriterArray::from_user_io_vecs(ctx, self.msg_iov, self.msg_iovlen as usize)
    }
}
