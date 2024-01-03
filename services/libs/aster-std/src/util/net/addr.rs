// SPDX-License-Identifier: MPL-2.0

use crate::net::iface::Ipv4Address;
use crate::net::socket::unix::UnixSocketAddr;
use crate::net::socket::SocketAddr;
use crate::prelude::*;
use crate::util::{read_bytes_from_user, read_val_from_user, write_val_to_user};

pub fn read_socket_addr_from_user(addr: Vaddr, addr_len: usize) -> Result<SocketAddr> {
    debug_assert!(addr_len >= core::mem::size_of::<SockAddr>());
    let sockaddr: SockAddr = read_val_from_user(addr)?;
    let socket_addr = match sockaddr.sa_family()? {
        SaFamily::AF_UNSPEC => {
            return_errno_with_message!(Errno::EINVAL, "the socket addr family is unspecified")
        }
        SaFamily::AF_UNIX => {
            debug_assert!(addr_len >= core::mem::size_of::<SockAddr>());
            let sa_family: u16 = read_val_from_user(addr)?;
            debug_assert!(sa_family == SaFamily::AF_UNIX as u16);

            let bytes = {
                let bytes_len = addr_len - core::mem::size_of::<u16>();
                let mut bytes = vec![0u8; bytes_len];
                read_bytes_from_user(addr + core::mem::size_of::<u16>(), &mut bytes)?;
                bytes
            };

            let unix_socket_addr = if bytes.starts_with(&[0]) {
                // Abstract unix socket addr
                let cstr = CStr::from_bytes_until_nul(&bytes[1..])?;
                let abstract_path = cstr.to_string_lossy().to_string();
                UnixSocketAddr::Abstract(abstract_path)
            } else {
                // Normal unix sockket addr
                let cstr = CStr::from_bytes_until_nul(&bytes)?;
                let path = cstr.to_string_lossy().to_string();
                UnixSocketAddr::Path(path)
            };

            SocketAddr::Unix(unix_socket_addr)
        }
        SaFamily::AF_INET => {
            debug_assert!(addr_len >= core::mem::size_of::<SockAddrInet>());
            let sock_addr_in: SockAddrInet = read_val_from_user(addr)?;
            SocketAddr::from(sock_addr_in)
        }
        SaFamily::AF_INET6 => {
            debug_assert!(addr_len >= core::mem::size_of::<SockAddrInet6>());
            let sock_addr_in6: SockAddrInet6 = read_val_from_user(addr)?;
            todo!()
        }
        _ => {
            return_errno_with_message!(Errno::EAFNOSUPPORT, "cannot support address for the family")
        }
    };
    Ok(socket_addr)
}

pub fn write_socket_addr_to_user(
    socket_addr: &SocketAddr,
    dest: Vaddr,
    addrlen_ptr: Vaddr,
) -> Result<()> {
    debug_assert!(addrlen_ptr != 0);
    if addrlen_ptr == 0 {
        return_errno_with_message!(Errno::EINVAL, "must provide the addrlen ptr");
    }
    let max_len = read_val_from_user::<i32>(addrlen_ptr)? as usize;
    let write_size = match socket_addr {
        SocketAddr::Unix(path) => {
            let sock_addr_unix = SockAddrUnix::try_from(path)?;
            let write_size = core::mem::size_of::<SockAddrUnix>();
            debug_assert!(max_len >= write_size);
            write_val_to_user(dest, &sock_addr_unix)?;
            write_size as i32
        }
        SocketAddr::IPv4(addr, port) => {
            let in_addr = InetAddr::from(*addr);
            let sock_addr_in = SockAddrInet::new(*port, in_addr);
            let write_size = core::mem::size_of::<SockAddrInet>();
            debug_assert!(max_len >= write_size);
            write_val_to_user(dest, &sock_addr_in)?;
            write_size as i32
        }
        SocketAddr::IPv6 => todo!(),
    };
    if addrlen_ptr != 0 {
        write_val_to_user(addrlen_ptr, &write_size)?;
    }
    Ok(())
}

/// PlaceHolder
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct SockAddr {
    sa_family: u16, // SaFamily
    sa_data: [u8; 14],
}

impl SockAddr {
    pub fn sa_family(&self) -> Result<SaFamily> {
        Ok(SaFamily::try_from(self.sa_family as i32)?)
    }
}

const SOCK_ADDR_UNIX_LEN: usize = 108;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct SockAddrUnix {
    sun_family: u16, // Always SaFamily::AF_UNIX
    sun_path: [u8; SOCK_ADDR_UNIX_LEN],
}

/// IPv4 4-byte address
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct InetAddr {
    s_addr: [u8; 4],
}

impl InetAddr {
    pub fn as_bytes(&self) -> &[u8] {
        &self.s_addr
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        debug_assert!(bytes.len() == 4);
        let mut s_addr = [0u8; 4];
        s_addr.copy_from_slice(bytes);
        Self { s_addr }
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct PortNum {
    port: [u8; 2],
}

impl PortNum {
    pub fn as_u16(&self) -> u16 {
        u16::from_be_bytes(self.port)
    }

    pub fn from_u16(value: u16) -> Self {
        let bytes = value.to_be_bytes();
        Self { port: bytes }
    }
}

/// IPv4 socket address
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct SockAddrInet {
    /// always SaFamily::AF_INET
    sin_family: u16,
    /// Port number
    sin_port_t: PortNum,
    /// IPv4 address
    sin_addr: InetAddr,
    /// Pad to size of 'SockAddr' structure (16 bytes)
    _pad: [u8; 8],
}

impl SockAddrInet {
    pub fn new(port: u16, addr: InetAddr) -> Self {
        let port = PortNum::from_u16(port);
        Self {
            sin_family: SaFamily::AF_INET as _,
            sin_port_t: port,
            sin_addr: addr,
            _pad: [0u8; 8],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct Inet6Addr {
    s6_addr: [u8; 16],
}

impl Inet6Addr {
    pub fn as_bytes(&self) -> &[u8] {
        &self.s6_addr
    }
}

/// IPv6 socket address
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct SockAddrInet6 {
    /// always SaFamily::AF_INET6
    sin6_family: u16,
    /// Port number
    sin6_port: PortNum,
    /// IPv6 flow information
    sin6_flowinfo: u32,
    /// IPv6 address
    sin6_addr: Inet6Addr,
    // Scope ID
    sin6_scope_id: u32,
}

/// Address family. The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/socket.h.
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum SaFamily {
    AF_UNSPEC = 0,
    AF_UNIX = 1, /* Unix domain sockets 		*/
    //AF_LOCAL	1	/* POSIX name for AF_UNIX	*/
    AF_INET = 2,      /* Internet IP Protocol 	*/
    AF_AX25 = 3,      /* Amateur Radio AX.25 		*/
    AF_IPX = 4,       /* Novell IPX 			*/
    AF_APPLETALK = 5, /* AppleTalk DDP 		*/
    AF_NETROM = 6,    /* Amateur Radio NET/ROM 	*/
    AF_BRIDGE = 7,    /* Multiprotocol bridge 	*/
    AF_ATMPVC = 8,    /* ATM PVCs			*/
    AF_X25 = 9,       /* Reserved for X.25 project 	*/
    AF_INET6 = 10,    /* IP version 6			*/
    AF_ROSE = 11,     /* Amateur Radio X.25 PLP	*/
    AF_DECnet = 12,   /* Reserved for DECnet project	*/
    AF_NETBEUI = 13,  /* Reserved for 802.2LLC project*/
    AF_SECURITY = 14, /* Security callback pseudo AF */
    AF_KEY = 15,      /* PF_KEY key management API */
    AF_NETLINK = 16,
    //AF_ROUTE	= AF_NETLINK /* Alias to emulate 4.4BSD */
    AF_PACKET = 17,     /* Packet family		*/
    AF_ASH = 18,        /* Ash				*/
    AF_ECONET = 19,     /* Acorn Econet			*/
    AF_ATMSVC = 20,     /* ATM SVCs			*/
    AF_RDS = 21,        /* RDS sockets 			*/
    AF_SNA = 22,        /* Linux SNA Project (nutters!) */
    AF_IRDA = 23,       /* IRDA sockets			*/
    AF_PPPOX = 24,      /* PPPoX sockets		*/
    AF_WANPIPE = 25,    /* Wanpipe API Sockets */
    AF_LLC = 26,        /* Linux LLC			*/
    AF_IB = 27,         /* Native InfiniBand address	*/
    AF_MPLS = 28,       /* MPLS */
    AF_CAN = 29,        /* Controller Area Network      */
    AF_TIPC = 30,       /* TIPC sockets			*/
    AF_BLUETOOTH = 31,  /* Bluetooth sockets 		*/
    AF_IUCV = 32,       /* IUCV sockets			*/
    AF_RXRPC = 33,      /* RxRPC sockets 		*/
    AF_ISDN = 34,       /* mISDN sockets 		*/
    AF_PHONET = 35,     /* Phonet sockets		*/
    AF_IEEE802154 = 36, /* IEEE802154 sockets		*/
    AF_CAIF = 37,       /* CAIF sockets			*/
    AF_ALG = 38,        /* Algorithm sockets		*/
    AF_NFC = 39,        /* NFC sockets			*/
    AF_VSOCK = 40,      /* vSockets			*/
    AF_KCM = 41,        /* Kernel Connection Multiplexor*/
    AF_QIPCRTR = 42,    /* Qualcomm IPC Router          */
    AF_SMC = 43,        /* smc sockets: reserve number for
                         * PF_SMC protocol family that
                         * reuses AF_INET address family
                         */
    AF_XDP = 44, /* XDP sockets			*/
    AF_MCTP = 45, /* Management component
                  * transport protocol
                  */
    AF_MAX = 46, /* For now.. */
}

impl From<InetAddr> for Ipv4Address {
    fn from(value: InetAddr) -> Self {
        let addr = value.as_bytes();
        Ipv4Address::from_bytes(addr)
    }
}

impl From<Ipv4Address> for InetAddr {
    fn from(value: Ipv4Address) -> Self {
        let bytes = value.as_bytes();
        InetAddr::from_bytes(bytes)
    }
}

impl From<SockAddrInet> for SocketAddr {
    fn from(value: SockAddrInet) -> Self {
        let port = value.sin_port_t.as_u16();
        let addr = Ipv4Address::from(value.sin_addr);
        SocketAddr::IPv4(addr, port)
    }
}

impl TryFrom<&UnixSocketAddr> for SockAddrUnix {
    type Error = Error;

    fn try_from(value: &UnixSocketAddr) -> Result<Self> {
        let mut sun_path = [0u8; SOCK_ADDR_UNIX_LEN];
        match value {
            UnixSocketAddr::Path(path) => {
                let bytes = path.as_bytes();
                let copy_len = bytes.len().min(SOCK_ADDR_UNIX_LEN - 1);
                sun_path[..copy_len].copy_from_slice(&bytes[..copy_len]);
                Ok(SockAddrUnix {
                    sun_family: SaFamily::AF_UNIX as u16,
                    sun_path,
                })
            }
            UnixSocketAddr::Abstract(_) => todo!(),
        }
    }
}
