use crate::net::iface::Ipv4Address;
use crate::net::socket::SocketAddr;
use crate::prelude::*;

#[macro_export]
macro_rules! get_socket_without_holding_filetable_lock {
    ($name:tt, $current: expr, $sockfd: expr) => {
        let file_like = {
            let file_table = $current.file_table().lock();
            file_table.get_file($sockfd)?.clone()
            // Drop filetable here to avoid locking
        };
        let $name = file_like
            .as_socket()
            .ok_or_else(|| Error::with_message(Errno::ENOTSOCK, "the file is not socket"))?;
    };
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
/// PlaceHolder
pub struct SockAddr {
    sa_family: u16, // SaFamily
    sa_data: [u8; 14],
}

impl SockAddr {
    pub fn sa_family(&self) -> Result<SaFamily> {
        Ok(SaFamily::try_from(self.sa_family as i32)?)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct SockAddrUn {
    sun_family: u16, // Always SaFamily::AF_UNIX
    sun_path: [u8; 108],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
/// IPv4 4-byte address
pub struct InAddr {
    s_addr: [u8; 4],
}

impl InAddr {
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

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
/// IPv4 socket address
pub struct SockAddrIn {
    /// always SaFamily::AF_INET
    sin_family: u16,
    /// Port number
    sin_port_t: PortNum,
    /// IPv4 address
    sin_addr: InAddr,
    /// Pad to size of 'SockAddr' structure (16 bytes)
    _pad: [u8; 8],
}

impl SockAddrIn {
    pub fn new(port: u16, addr: InAddr) -> Self {
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
/// IPv6 address
pub struct In6Addr {
    s6_addr: [u8; 16],
}

impl In6Addr {
    pub fn as_bytes(&self) -> &[u8] {
        &self.s6_addr
    }
}

/// IPv6 socket address
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct SockAddrIn6 {
    /// always SaFamily::AF_INET6
    sin6_family: u16,
    /// Port number
    sin6_port: PortNum,
    /// IPv6 flow information
    sin6_flowinfo: u32,
    /// IPv6 address
    sin6_addr: In6Addr,
    // Scope ID
    sin6_scope_id: u32,
}

/// Address family. The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/socket.h.
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
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

impl From<InAddr> for Ipv4Address {
    fn from(value: InAddr) -> Self {
        let addr = value.as_bytes();
        Ipv4Address::from_bytes(addr)
    }
}

impl From<Ipv4Address> for InAddr {
    fn from(value: Ipv4Address) -> Self {
        let bytes = value.as_bytes();
        InAddr::from_bytes(bytes)
    }
}

impl From<SockAddrIn> for SocketAddr {
    fn from(value: SockAddrIn) -> Self {
        let port = value.sin_port_t.as_u16();
        let addr = Ipv4Address::from(value.sin_addr);
        SocketAddr::IPv4(addr, port)
    }
}
