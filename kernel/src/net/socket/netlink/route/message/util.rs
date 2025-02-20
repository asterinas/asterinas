// SPDX-License-Identifier: MPL-2.0

use super::CMessageType;
use crate::{
    net::socket::netlink::message::{CNetlinkMessageHeader, NetlinkMessageCommonFlags},
    prelude::*,
    util::MultiWrite,
};

/// Net device type.
/// Reference: https://elixir.bootlin.com/linux/v6.0.18/source/include/uapi/linux/if_arp.h#L30
#[repr(u16)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq)]
pub enum NetDeviceType {
    // Arp protocol hardware identifiers
    /// from KA9Q: NET/ROM pseudo
    NETROM = 0,
    /// Ethernet 10Mbps
    ETHER = 1,
    /// Experimental Ethernet
    EETHER = 2,

    // Dummy types for non ARP hardware
    /// IPIP tunnel
    TUNNEL = 768,
    /// IP6IP6 tunnel
    TUNNEL6 = 769,
    /// Frame Relay Access Device
    FRAD = 770,
    /// SKIP vif
    SKIP = 771,
    /// Loopback device
    LOOPBACK = 772,
    /// Localtalk device
    LOCALTALK = 773,
    // TODO: This enum is not exhaustive
}

bitflags! {
    /// Net device flags.
    /// Referenece: https://elixir.bootlin.com/linux/v6.0.18/source/include/uapi/linux/if.h#L82
    pub struct NetDeviceFlags: u32 {
        /// Interface is up
        const UP				= 1<<0;
        /// Broadcast address valid
        const BROADCAST			= 1<<1;
        /// Turn on debugging
        const DEBUG			    = 1<<2;
        /// Loopback net
        const LOOPBACK			= 1<<3;
        /// Interface is has p-p link
        const POINTOPOINT		= 1<<4;
        /// Avoid use of trailers
        const NOTRAILERS		= 1<<5;
        /// Interface RFC2863 OPER_UP
        const RUNNING			= 1<<6;
        /// No ARP protocol
        const NOARP			    = 1<<7;
        /// Receive all packets
        const PROMISC			= 1<<8;
        /// Receive all multicast packets
        const ALLMULTI			= 1<<9;
        /// Master of a load balancer
        const MASTER			= 1<<10;
        /// Slave of a load balancer
        const SLAVE			    = 1<<11;
        /// Supports multicast
        const MULTICAST			= 1<<12;
        /// Can set media type
        const PORTSEL			= 1<<13;
        /// Auto media select active
        const AUTOMEDIA			= 1<<14;
        /// Dialup device with changing addresses
        const DYNAMIC			= 1<<15;
        /// Driver signals L1 up
        const LOWER_UP			= 1<<16;
        /// Driver signals dormant
        const DORMANT			= 1<<17;
        /// Echo sent packets
        const ECHO			    = 1<<18;
    }
}

#[derive(Debug)]
pub struct DoneMessage {
    pub error_code: i32,
    pub seq: u32,
    pub pid: u32,
    // TODO: Optional extented ACK(Ref: https://docs.kernel.org/userspace-api/netlink/intro.html#ext-ack)
}

#[derive(Debug)]
pub struct ErrorMessage {
    error_code: i32,
    request_header: CNetlinkMessageHeader,
    // TDOD: Optional payload of request
    // TODO: Optional extented ACK(Ref: https://docs.kernel.org/userspace-api/netlink/intro.html#ext-ack)
}

#[derive(Debug)]
pub enum AckMessage {
    Done(DoneMessage),
    Error(ErrorMessage),
}

impl AckMessage {
    pub const fn new_done(error_code: i32, request_header: &CNetlinkMessageHeader) -> AckMessage {
        AckMessage::Done(DoneMessage {
            error_code,
            seq: request_header.seq,
            pid: request_header.pid,
        })
    }

    pub const fn new_error(errno: Errno, request_header: CNetlinkMessageHeader) -> AckMessage {
        AckMessage::Error(ErrorMessage {
            error_code: -(errno as i32),
            request_header,
        })
    }

    pub fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
        println!("ack message = {:?}", self);

        match self {
            AckMessage::Done(done_message) => done_message.write_to_user(writer),
            AckMessage::Error(error_message) => error_message.write_to_user(writer),
        }
    }
}

impl DoneMessage {
    fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
        let written_len = core::mem::size_of::<CNetlinkMessageHeader>() + size_of::<i32>();

        let header = CNetlinkMessageHeader {
            len: written_len as _,
            type_: CMessageType::DONE as _,
            flags: NetlinkMessageCommonFlags::MULTI.bits(),
            seq: self.seq,
            pid: self.pid,
        };

        let mut total_len = 0;

        writer.write_val(&header)?;
        total_len += core::mem::size_of_val(&header);

        writer.write_val(&self.error_code)?;
        total_len += core::mem::size_of_val(&self.error_code);

        debug_assert_eq!(written_len, total_len);
        Ok(total_len)
    }
}

impl ErrorMessage {
    fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
        let written_len = core::mem::size_of::<CNetlinkMessageHeader>() * 2 + size_of::<i32>();

        let header = CNetlinkMessageHeader {
            len: written_len as _,
            type_: CMessageType::ERROR as _,
            flags: NetlinkMessageCommonFlags::empty().bits(),
            seq: self.request_header.seq,
            pid: self.request_header.pid,
        };

        let mut total_len = 0;

        writer.write_val(&header)?;
        total_len += size_of_val(&header);

        writer.write_val(&self.error_code)?;
        total_len += size_of_val(&self.error_code);

        writer.write_val(&self.request_header)?;
        total_len += size_of_val(&self.request_header);

        debug_assert_eq!(total_len, written_len);
        Ok(total_len)
    }
}
