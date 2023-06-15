use crate::fs::file_handle::FileLike;
use crate::net::socket::ip::DatagramSocket;
use crate::net::socket::ip::StreamSocket;
use crate::util::net::SaFamily;
use crate::{log_syscall_entry, prelude::*};

use super::SyscallReturn;
use super::SYS_SOCKET;

pub fn sys_socket(domain: i32, type_: i32, protocol: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SOCKET);
    let domain = SaFamily::try_from(domain)?;
    let sock_type = SockType::try_from(type_ & SOCK_TYPE_MASK)?;
    let sock_flags = SockFlags::from_bits_truncate(type_ & !SOCK_TYPE_MASK);
    let protocol = Protocol::try_from(protocol)?;
    debug!(
        "domain = {:?}, sock_type = {:?}, sock_flags = {:?}, protocol = {:?}",
        domain, sock_type, sock_flags, protocol
    );
    let nonblocking = sock_flags.contains(SockFlags::SOCK_NONBLOCK);
    let file_like = match (domain, sock_type, protocol) {
        (
            SaFamily::AF_INET,
            SockType::SOCK_STREAM,
            Protocol::IPPROTO_IP | Protocol::IPPROTO_TCP,
        ) => Arc::new(StreamSocket::new(nonblocking)) as Arc<dyn FileLike>,
        (SaFamily::AF_INET, SockType::SOCK_DGRAM, Protocol::IPPROTO_IP | Protocol::IPPROTO_UDP) => {
            Arc::new(DatagramSocket::new(nonblocking)) as Arc<dyn FileLike>
        }
        _ => return_errno_with_message!(Errno::EAFNOSUPPORT, "unsupported domain"),
    };
    let fd = {
        let current = current!();
        let mut file_table = current.file_table().lock();
        file_table.insert(file_like)
    };
    Ok(SyscallReturn::Return(fd as _))
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
/// Standard well-defined IP protocols.
/// From https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/in.h.
enum Protocol {
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

#[repr(i32)]
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, TryFromInt)]
/// Socket types.
/// From https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/net.h
enum SockType {
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

const SOCK_TYPE_MASK: i32 = 0xf;

bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    struct SockFlags: i32 {
        const SOCK_NONBLOCK = 1 << 11;
        const SOCK_CLOEXEC = 1 << 19;
    }
}
